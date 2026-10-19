//! Analyse de `robots.txt` **conforme à la RFC 9309**.
//!
//! - jokers `*` (n'importe quelle suite) et `$` (fin de chemin) ;
//! - **règle du plus long match** ;
//! - à longueur égale, `Allow` l'emporte sur `Disallow` ;
//! - sélection du **groupe d'agent le plus spécifique**, sinon `*` ;
//! - extensions `Crawl-delay` et `Request-rate` (X/Y) ;
//! - directive globale `Sitemap`.

use regex::Regex;

/// Un motif `Allow`/`Disallow` compilé.
#[derive(Debug, Clone)]
pub struct Pattern {
    regex: Regex,
    /// Longueur comparée pour la règle du plus long match.
    weight: usize,
}

impl Pattern {
    /// Compile un motif. Un motif vide (`Disallow:`) ne bloque rien.
    fn new(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }

        let raw = normalize_path(raw);
        let anchored = raw.ends_with('$');
        let body = if anchored {
            &raw[..raw.len() - 1]
        } else {
            &raw
        };

        let mut expression = String::from("^");
        for ch in body.chars() {
            match ch {
                '*' => expression.push_str(".*"),
                c => expression.push_str(&regex::escape(&c.to_string())),
            }
        }
        if anchored {
            expression.push('$');
        }

        Some(Self {
            regex: Regex::new(&expression).ok()?,
            weight: body.len(),
        })
    }

    fn matches(&self, path: &str) -> bool {
        self.regex.is_match(path)
    }
}

/// Longueur du plus long motif qui correspond (`None` si aucun).
fn longest_match(patterns: &[Pattern], path: &str) -> Option<usize> {
    patterns
        .iter()
        .filter(|p| p.matches(path))
        .map(|p| p.weight)
        .max()
}

/// Règles applicables à un agent donné.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    pub allow: Vec<Pattern>,
    pub disallow: Vec<Pattern>,
    /// Délai demandé par le site, en millisecondes.
    pub crawl_delay_ms: Option<u64>,
    /// `Request-rate: X/Y` → (requêtes autorisées, période en secondes).
    pub request_rate: Option<(usize, u64)>,
    pub sitemaps: Vec<String>,
}

impl Rules {
    /// Vrai si `path` (chemin + requête) est autorisé.
    pub fn allows(&self, path: &str) -> bool {
        let path = normalize_path(path);
        if path == "/robots.txt" {
            return true;
        }
        let denied = longest_match(&self.disallow, &path);
        let allowed = longest_match(&self.allow, &path);
        match (denied, allowed) {
            // Motif le plus long gagnant ; à égalité, Allow l'emporte.
            (_, Some(len)) if denied.map(|d| len >= d).unwrap_or(true) => true,
            (Some(_), _) => false,
            _ => true,
        }
    }

    /// Vrai si aucune restriction n'est déclarée.
    pub fn is_permissive(&self) -> bool {
        self.allow.is_empty() && self.disallow.is_empty()
    }
}

/// Compare escaped and unescaped unreserved bytes consistently (RFC 9309).
fn normalize_path(path: &str) -> String {
    let mut result = String::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'%'
            && i + 2 < bytes.len()
            && let Ok(pair) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(decoded) = u8::from_str_radix(pair, 16)
        {
            if decoded.is_ascii_alphanumeric() || b"-._~".contains(&decoded) {
                result.push(decoded as char);
            } else {
                result.push_str(&format!("%{decoded:02X}"));
            }
            i += 3;
            continue;
        }
        if byte.is_ascii() {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
        i += 1;
    }
    result
}

/// Convertit `Crawl-delay` en millisecondes (accepte les décimales).
fn parse_delay(value: &str) -> Option<u64> {
    let seconds: f64 = value.trim().parse().ok()?;
    if seconds.is_finite() && seconds >= 0.0 {
        Some((seconds * 1000.0).round() as u64)
    } else {
        None
    }
}

/// Analyse `Request-rate: X/Y` (X requêtes toutes les Y secondes).
fn parse_request_rate(value: &str) -> Option<(usize, u64)> {
    let (requests, period) = value.split_once('/')?;
    let requests: usize = requests.trim().parse().ok()?;
    let seconds: u64 = period.trim().parse().ok()?;
    (requests > 0 && seconds > 0).then_some((requests, seconds))
}

/// Un groupe `User-agent:` et ses directives.
struct Group {
    agents: Vec<String>,
    entries: Vec<(String, String)>,
}

/// Analyse un `robots.txt` pour l'agent donné.
pub fn parse(body: &str, user_agent: &str) -> Rules {
    let wanted = user_agent.to_ascii_lowercase();
    let mut groups: Vec<Group> = Vec::new();
    let mut sitemaps: Vec<String> = Vec::new();

    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_string();

        match key.as_str() {
            "user-agent" => {
                let agent = value.to_ascii_lowercase();
                // Tant qu'aucune règle n'a été vue, on complète le groupe courant.
                let extendable = groups.last().map(|g| g.entries.is_empty()).unwrap_or(false);
                if extendable {
                    groups.last_mut().expect("groupe").agents.push(agent);
                } else {
                    groups.push(Group {
                        agents: vec![agent],
                        entries: Vec::new(),
                    });
                }
            }
            "sitemap" => sitemaps.push(value),
            "disallow" | "allow" | "crawl-delay" | "request-rate" => {
                if groups.is_empty() {
                    groups.push(Group {
                        agents: vec!["*".to_string()],
                        entries: Vec::new(),
                    });
                }
                groups
                    .last_mut()
                    .expect("groupe")
                    .entries
                    .push((key, value));
            }
            _ => {}
        }
    }

    // Groupe le plus spécifique correspondant à notre agent, sinon « * ».
    let weights: Vec<usize> = groups
        .iter()
        .map(|group| {
            group
                .agents
                .iter()
                .filter(|agent| {
                    !agent.is_empty() && agent.as_str() != "*" && wanted.contains(agent.as_str())
                })
                .map(String::len)
                .max()
                .unwrap_or(0)
        })
        .collect();
    let specific_len = weights.iter().copied().max().unwrap_or(0);

    let mut rules = Rules {
        sitemaps,
        ..Rules::default()
    };

    for (group, weight) in groups.iter().zip(weights) {
        let selected = if specific_len > 0 {
            weight == specific_len
        } else {
            group.agents.iter().any(|agent| agent == "*")
        };
        if !selected {
            continue;
        }
        for (key, value) in &group.entries {
            match key.as_str() {
                "disallow" => {
                    if let Some(pattern) = Pattern::new(value) {
                        rules.disallow.push(pattern);
                    }
                }
                "allow" => {
                    if let Some(pattern) = Pattern::new(value) {
                        rules.allow.push(pattern);
                    }
                }
                "crawl-delay" => {
                    if let Some(ms) = parse_delay(value) {
                        rules.crawl_delay_ms = Some(rules.crawl_delay_ms.unwrap_or(0).max(ms));
                    }
                }
                "request-rate" => {
                    if let Some((requests, seconds)) = parse_request_rate(value) {
                        let delay = seconds.saturating_mul(1000).div_ceil(requests as u64);
                        rules.crawl_delay_ms = Some(rules.crawl_delay_ms.unwrap_or(0).max(delay));
                        rules.request_rate = Some((requests, seconds));
                    }
                }
                _ => {}
            }
        }
    }

    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorter_allow_cannot_override_longer_disallow() {
        let rules = parse(
            "User-agent: *\nAllow: /\nDisallow: /private\n",
            "VertexCrawler",
        );
        assert!(!rules.allows("/private/data"));
        assert!(rules.allows("/public"));
    }

    #[test]
    fn combines_matching_groups_and_accepts_empty_specific_group() {
        let rules = parse(
            "User-agent: VertexCrawler\nDisallow: /one\nUser-agent: vertexcrawler\nDisallow: /two\n",
            "VertexCrawler",
        );
        assert!(!rules.allows("/one"));
        assert!(!rules.allows("/two"));
        assert!(
            parse(
                "User-agent: *\nDisallow: /\nUser-agent: VertexCrawler\n",
                "VertexCrawler"
            )
            .allows("/one")
        );
    }

    #[test]
    fn escaped_paths_and_allow_ties_follow_the_same_rules() {
        let rules = parse(
            "User-agent: *\nDisallow: /café\nDisallow: /~private\nDisallow: /same\nAllow: /same\n",
            "VertexCrawler",
        );
        assert!(!rules.allows("/caf%C3%A9"));
        assert!(!rules.allows("/%7eprivate"));
        assert!(rules.allows("/same"));
    }

    /// Exemples de la RFC 9309 : la règle du plus long match.
    #[test]
    fn rfc_longest_match_wins() {
        let rules = parse(
            "User-agent: *\nDisallow: /fish\nDisallow: /birds\nAllow: /birds/fish\n",
            "VertexCrawler",
        );
        assert!(!rules.allows("/fish"), "/fish bloqué");
        assert!(!rules.allows("/birds"), "/birds bloqué");
        assert!(
            rules.allows("/birds/fish"),
            "/birds/fish autorisé (plus long)"
        );
        assert!(rules.allows("/birds/fishheads"));
        assert!(!rules.allows("/fishheads"), "commence par /fish");
    }

    #[test]
    fn wildcard_and_end_anchor() {
        let rules = parse(
            "User-agent: *\nDisallow: /fish*heads\nDisallow: /molluscs$\n",
            "VertexCrawler",
        );
        assert!(!rules.allows("/fishheads"), "joker *");
        // Les motifs sont ancrés AU DÉBUT du chemin : /fish*heads ne bloque
        // que ce qui commence par /fish.
        assert!(!rules.allows("/fish/some/heads"), "joker au milieu");
        assert!(rules.allows("/cat/fish/heads"), "motif ancré au début");
        assert!(!rules.allows("/molluscs"), "motif ancré $");
        assert!(rules.allows("/molluscs/slug"), "$ ancre la fin exacte");
    }

    #[test]
    fn most_specific_user_agent_group_wins() {
        let body = "User-agent: *\nDisallow: /\n\nUser-agent: VertexCrawler\nDisallow:\n";
        assert!(
            parse(body, "VertexCrawler").allows("/"),
            "agent spécifique non bloqué"
        );
        assert!(
            !parse(body, "AutreBot").allows("/"),
            "agent générique bloqué"
        );
    }

    #[test]
    fn empty_disallow_allows_everything() {
        let rules = parse("User-agent: *\nDisallow:\n", "VertexCrawler");
        assert!(rules.is_permissive());
        assert!(rules.allows("/nimporte/quoi"));
    }

    #[test]
    fn crawl_delay_and_sitemaps() {
        let rules = parse(
            "User-agent: *\nCrawl-delay: 1.5\nSitemap: https://example.com/sitemap.xml\n\nSitemap: https://example.com/s.xml\n",
            "VertexCrawler",
        );
        assert_eq!(rules.crawl_delay_ms, Some(1500));
        assert_eq!(rules.sitemaps.len(), 2);
    }

    #[test]
    fn comments_are_ignored() {
        let rules = parse(
            "# commentaire\n\nUser-agent: *   # agent\n\nDisallow: /prive\n",
            "VertexCrawler",
        );
        assert!(!rules.allows("/prive"));
        assert!(rules.allows("/public"));
    }
}
