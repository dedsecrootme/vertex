import { DatabaseSync, backup } from 'node:sqlite';
import { mkdir, stat, statfs, access } from 'node:fs/promises';
import { resolve, dirname } from 'node:path';
import { parseArgs } from 'node:util';

const {positionals:[input,output],values} = parseArgs({allowPositionals:true,options:{'reserve-mib':{type:'string',default:'5120'}}});
if (!input || !output) throw new Error('Usage: node scripts/backup-index.mjs source.db snapshot.db [--reserve-mib 51200]');
const reserve=Number(values['reserve-mib']);
if(!Number.isSafeInteger(reserve)||reserve<0)throw new Error('Reserve invalide');
const source = resolve(input);
const destination = resolve(output);
if (source === destination) throw new Error('Source et sauvegarde doivent etre distinctes');
let exists = false;
try { await access(destination); exists = true; } catch (error) { if (error.code !== 'ENOENT') throw error; }
if (exists) throw new Error('La sauvegarde existe deja : aucun ecrasement');
const file = await stat(source);
await mkdir(dirname(destination), { recursive: true });
const disk = await statfs(dirname(destination));
if (disk.bavail * disk.bsize < file.size + reserve * 1024 ** 2) throw new Error('Espace insuffisant sur le disque de destination');
const db = new DatabaseSync(source, { readOnly: true });
let previous = 0;
try {
  db.exec('PRAGMA busy_timeout=10000; BEGIN');
  // Pin one WAL snapshot so concurrent crawl writes do not restart a large backup.
  db.prepare('SELECT name FROM sqlite_schema LIMIT 1').get();
  await backup(db, destination, { rate: 2000, progress: ({ totalPages, remainingPages }) => {
    if (Date.now() - previous > 5000 || remainingPages === 0) {
      console.log(JSON.stringify({ backup: destination, totalPages, remainingPages }));
      previous = Date.now();
    }
  } });
  db.exec('COMMIT');
  console.log(JSON.stringify({ complete: true, bytes: (await stat(destination)).size, backup: destination }));
} finally { db.close(); }
