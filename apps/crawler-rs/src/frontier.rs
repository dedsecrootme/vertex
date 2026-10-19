use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

const BATCH: i64 = 256;
const CAPACITY: usize = 4096;
const PER_HOST: usize = 32;

#[derive(Debug)]
pub(crate) struct Claim {
    pub url: String,
    pub depth: i64,
    pub host: String,
    id: i64,
}

#[derive(Default)]
pub(crate) struct Frontier {
    candidates: VecDeque<Claim>,
    cached_ids: HashSet<i64>,
    per_host: HashMap<String, usize>,
    priority: Option<i64>,
    after_id: i64,
    resume_at: Option<Instant>,
}

impl Frontier {
    fn remove(&mut self, index: usize) -> Claim {
        let claim = self.candidates.remove(index).unwrap();
        self.cached_ids.remove(&claim.id);
        let count = self.per_host.get_mut(&claim.host).unwrap();
        *count -= 1;
        if *count == 0 {
            self.per_host.remove(&claim.host);
        }
        claim
    }

    // Keyset pagination visits each priority once per pass; no OFFSET or frontier-wide sort.
    fn refill(&mut self, conn: &Connection, max_depth: i64) -> rusqlite::Result<()> {
        if self.resume_at.is_some_and(|at| at > Instant::now()) {
            return Ok(());
        }
        if self.priority.is_none() {
            self.priority = conn.query_row(
                "SELECT priority FROM urls INDEXED BY idx_urls_claim WHERE status='pending' AND safety!='malicious' AND attempts<3 ORDER BY priority DESC,id ASC LIMIT 1",
                [], |row| row.get(0),
            ).optional()?;
            self.after_id = 0;
        }
        let Some(priority) = self.priority else {
            self.resume_at = Some(Instant::now() + Duration::from_millis(500));
            return Ok(());
        };
        let rows = {
            let mut query = conn.prepare_cached(
                "SELECT id,url,depth,next_attempt FROM urls INDEXED BY idx_urls_claim WHERE status='pending' AND safety!='malicious' AND attempts<3 AND priority=?1 AND id>?2 ORDER BY id ASC LIMIT ?3",
            )?;
            query
                .query_map(params![priority, self.after_id, BATCH], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if rows.is_empty() {
            self.priority = conn.query_row(
                "SELECT priority FROM urls INDEXED BY idx_urls_claim WHERE status='pending' AND safety!='malicious' AND attempts<3 AND priority<?1 ORDER BY priority DESC,id ASC LIMIT 1",
                [priority], |row| row.get(0),
            ).optional()?;
            self.after_id = 0;
            if self.priority.is_none() {
                self.resume_at = Some(Instant::now() + Duration::from_millis(500));
            }
            return Ok(());
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        for (id, url, depth, retry) in rows {
            self.after_id = id;
            if depth > max_depth || retry > now as i64 || self.cached_ids.contains(&id) {
                continue;
            }
            let host = url::Url::parse(&url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_default();
            if self.per_host.get(&host).copied().unwrap_or(0) >= PER_HOST {
                continue;
            }
            if self.candidates.len() >= CAPACITY {
                self.remove(0);
            }
            *self.per_host.entry(host.clone()).or_default() += 1;
            self.cached_ids.insert(id);
            self.candidates.push_back(Claim {
                id,
                url,
                depth,
                host,
            });
        }
        Ok(())
    }

    pub fn claim(
        &mut self,
        conn: &Connection,
        max_depth: i64,
        active: &HashSet<String>,
        next: &HashMap<String, Instant>,
    ) -> rusqlite::Result<Option<Claim>> {
        for _ in 0..4 {
            let now = Instant::now();
            while let Some(index) = self.candidates.iter().position(|row| {
                !active.contains(&row.host) && next.get(&row.host).is_none_or(|at| *at <= now)
            }) {
                let claim = self.remove(index);
                // Cached candidates are not reservations: another writer can have excluded one.
                let changed = crate::database::retry_busy(|| {
                    conn.prepare_cached(
                    "UPDATE urls SET status='crawling',attempts=attempts+1,updated_at=CURRENT_TIMESTAMP WHERE id=?1 AND url=?2 AND status='pending' AND safety!='malicious' AND attempts<3 AND depth<=?3 AND next_attempt<=unixepoch()",
                )?.execute(params![claim.id, claim.url, max_depth])
                })?;
                if changed == 1 {
                    return Ok(Some(claim));
                }
            }
            self.refill(conn, max_depth)?;
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE urls(id INTEGER PRIMARY KEY,url TEXT UNIQUE,depth INTEGER DEFAULT 0,status TEXT DEFAULT 'pending',safety TEXT DEFAULT 'safe',priority INTEGER DEFAULT 0,attempts INTEGER DEFAULT 0,updated_at TEXT,next_attempt INTEGER DEFAULT 0); CREATE INDEX idx_urls_claim ON urls(priority DESC,id ASC) WHERE status='pending' AND safety!='malicious' AND attempts<3;").unwrap();
        conn
    }
    #[test]
    fn advances_past_a_large_busy_host_without_claiming_its_urls() {
        let conn = db();
        for id in 0..2000 {
            conn.execute(
                "INSERT INTO urls(url) VALUES(?1)",
                [format!("https://busy.example/{id}")],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO urls(url) VALUES('https://ready.example/article')",
            [],
        )
        .unwrap();
        let mut frontier = Frontier::default();
        let active = HashSet::from(["busy.example".into()]);
        let mut found = None;
        for _ in 0..8 {
            if let Some(claim) = frontier.claim(&conn, 20, &active, &HashMap::new()).unwrap() {
                found = Some(claim);
                break;
            }
        }
        assert_eq!(found.unwrap().host, "ready.example");
        assert_eq!(
            conn.query_row(
                "SELECT sum(attempts) FROM urls WHERE url LIKE 'https://busy.example/%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert!(frontier.candidates.len() <= CAPACITY);
    }
    #[test]
    fn cooldown_and_priority_are_preserved_and_stale_candidates_are_rechecked() {
        let conn = db();
        conn.execute_batch("INSERT INTO urls(url,priority) VALUES('https://slow.example/a',10),('https://ready.example/a',0),('https://other.example/a',0);").unwrap();
        let mut frontier = Frontier::default();
        let next = HashMap::from([(
            "slow.example".into(),
            Instant::now() + Duration::from_secs(3600),
        )]);
        let first = frontier
            .claim(&conn, 20, &HashSet::new(), &next)
            .unwrap()
            .unwrap();
        assert_eq!(first.host, "ready.example");
        conn.execute(
            "UPDATE urls SET status='skipped' WHERE url='https://other.example/a'",
            [],
        )
        .unwrap();
        assert!(
            frontier
                .claim(&conn, 20, &HashSet::new(), &next)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            conn.query_row(
                "SELECT attempts FROM urls WHERE url='https://slow.example/a'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
    #[test]
    fn keyset_reads_use_the_claim_index_without_sorting() {
        let conn = db();
        let mut query=conn.prepare("EXPLAIN QUERY PLAN SELECT id,url,depth,next_attempt FROM urls INDEXED BY idx_urls_claim WHERE status='pending' AND safety!='malicious' AND attempts<3 AND priority=0 AND id>10 ORDER BY id ASC LIMIT 256").unwrap();
        let plan = query
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(plan.iter().any(|line| line.contains("idx_urls_claim")));
        assert!(!plan.iter().any(|line| line.contains("TEMP B-TREE")));
    }
    #[test]
    fn future_retries_and_deep_urls_are_not_claimed() {
        let conn = db();
        conn.execute_batch("INSERT INTO urls(url,next_attempt) VALUES('https://future.example/a',unixepoch()+3600); INSERT INTO urls(url,depth) VALUES('https://deep.example/a',99); INSERT INTO urls(url) VALUES('https://ready.example/a');").unwrap();
        let claim = Frontier::default()
            .claim(&conn, 6, &HashSet::new(), &HashMap::new())
            .unwrap()
            .unwrap();
        assert_eq!(claim.host, "ready.example");
        assert_eq!(
            conn.query_row(
                "SELECT sum(attempts) FROM urls WHERE status='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
