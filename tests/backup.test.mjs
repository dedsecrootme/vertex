import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { mkdtemp, rm, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';

const run = promisify(execFile);
test('online backup preserves content and refuses overwrite or same source', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'vertex-backup-'));
  const input = join(dir, 'index.db'), output = join(dir, 'snapshot.db');
  const script = resolve('scripts/backup-index.mjs');
  let source, snapshot;
  try {
    source = new DatabaseSync(input);
    source.exec("PRAGMA journal_mode=WAL; CREATE TABLE pages(title TEXT); INSERT INTO pages VALUES('Page originale')");
    await run(process.execPath, [script, input, output, '--reserve-mib', '0'], { windowsHide: true });
    snapshot = new DatabaseSync(output, { readOnly: true });
    assert.equal(snapshot.prepare('SELECT title FROM pages').get().title, 'Page originale');
    snapshot.close(); snapshot = null;
    const before = await readFile(output);
    await assert.rejects(run(process.execPath, [script, input, output, '--reserve-mib', '0']), /aucun ecrasement/);
    assert.deepEqual(await readFile(output), before);
    await assert.rejects(run(process.execPath, [script, input, input]), /distinctes/);
    assert.equal(source.prepare('SELECT count(*) AS n FROM pages').get().n, 1);
  } finally {
    snapshot?.close(); source?.close();
    await rm(dir, { recursive: true, force: true });
  }
});
