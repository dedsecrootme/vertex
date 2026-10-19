import { DatabaseSync } from 'node:sqlite';
import { writeFile, rename } from 'node:fs/promises';
import { resolve, dirname, join } from 'node:path';

const path = resolve(process.argv[2] || 'database/vertex.db');
const db = new DatabaseSync(path, { readOnly: true });
const started = Date.now();
const domains = new Map(), depths = new Map(), days = new Map(), batches = new Map(), statuses = new Map(), parameters = new Map();
let total = 0, withQuery = 0, fragments = 0, invalid = 0;
const bump = (map, key) => map.set(key, (map.get(key) || 0) + 1);
const top = (map, limit = 12) => [...map].sort((a, b) => b[1] - a[1]).slice(0, limit).map(([label, count]) => ({ label, count }));
try {
  for (const row of db.prepare('SELECT url,domain,depth,status,created_at FROM urls').iterate()) {
    total++;
    bump(depths, String(row.depth)); bump(statuses, row.status);
    bump(days, String(row.created_at || '').slice(0, 10));
    bump(batches, String(row.created_at || '').slice(0, 16));
    try {
      const url = new URL(row.url);
      bump(domains, url.hostname.toLowerCase());
      if (url.search) { withQuery++; for (const key of new Set(url.searchParams.keys())) bump(parameters, key.slice(0, 100)); }
      if (url.hash) fragments++;
    } catch { invalid++; }
    if (total % 250000 === 0) console.log(JSON.stringify({ scanned: total, elapsedSeconds: (Date.now() - started) / 1000 }));
  }
  const result = { completedAt: new Date().toISOString(), elapsedSeconds: (Date.now() - started) / 1000,
    total, uniqueDomains: domains.size, withQuery, fragments, invalid,
    topDomains: top(domains), depths: top(depths, 100), statuses: top(statuses), days: top(days), batches: top(batches), parameters: top(parameters),
    exactUrlDeduplication: !!db.prepare("SELECT 1 FROM pragma_index_list('urls') WHERE origin='u' AND \"unique\"=1").get(),
    provenanceAvailable: false };
  const output = join(dirname(path), 'queue-audit.json');
  await writeFile(output + '.tmp', JSON.stringify(result, null, 2));
  await rename(output + '.tmp', output);
  console.log(JSON.stringify(result));
} finally { db.close(); }
