import {test} from 'node:test';
import assert from 'node:assert/strict';
import {DatabaseSync} from 'node:sqlite';
import {mkdtemp,rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {Worker} from 'node:worker_threads';
import {once} from 'node:events';
import {exclusionReason,cleanQueue} from '../scripts/clean-queue.mjs';

test('cleanup policy matches crawler exclusions and preserves public articles',()=>{
  for(const url of ['https://en.wikipedia.org/w/index.php?title=X&action=edit','https://wikidata.org/?redlink=1','https://fr.wiktionary.org/?veaction=history'])assert.ok(exclusionReason(url));
  for(const url of ['https://en.wikipedia.org/wiki/Rust','https://en.wikipedia.org/?action=view','https://example.com/?action=edit','https://wikipedia.org.example.com/?action=edit','https://en.wikipedia.org/?redlink=0','not an url'])assert.equal(exclusionReason(url),null);
});

test('cleanup changes pending exclusions only and preserves rows, pages and history',async()=>{
  const dir=await mkdtemp(join(tmpdir(),'vertex-cleanup-'));
  const path=join(dir,'vertex.db');let db=new DatabaseSync(path);
  try{
    db.exec("CREATE TABLE urls(id INTEGER PRIMARY KEY,url TEXT,status TEXT,error TEXT,updated_at TEXT); CREATE TABLE pages(id INTEGER PRIMARY KEY,content TEXT); INSERT INTO pages VALUES(1,'keep');");
    const insert=db.prepare('INSERT INTO urls(id,url,status) VALUES(?,?,?)');
    insert.run(1,'https://en.wikipedia.org/?action=edit','pending');
    insert.run(2,'https://en.wikipedia.org/wiki/Rust','pending');
    insert.run(3,'https://en.wikipedia.org/?redlink=1','done');
    insert.run(4,'https://en.wikipedia.org/?veaction=edit','crawling');
    insert.run(5,'https://example.com/?action=edit','pending');
    insert.run(6,'https://fr.wikipedia.org/?redlink=1','pending');
    db.close();db=null;
    const result=await cleanQueue(path,{batchSize:2,deadline:new Date(Date.now()+30000)});
    assert.equal(result.excluded,2);
    db=new DatabaseSync(path);
    assert.deepEqual(db.prepare('SELECT status FROM urls ORDER BY id').all().map(row=>row.status),['skipped','pending','done','crawling','pending','skipped']);
    assert.equal(db.prepare('SELECT content FROM pages').get().content,'keep');
    assert.equal(db.prepare('SELECT count(*) AS n FROM urls').get().n,6);
    db.close();db=null;
    const resumed=await cleanQueue(path,{deadline:new Date(Date.now()+30000)});
    assert.equal(resumed.excluded,0);assert.equal(resumed.totalExcluded,2);
  }finally{db?.close();await rm(dir,{recursive:true,force:true});}
});

test('cleanup waits through a long write lock and keeps its checkpoint', {timeout:40000}, async()=>{
  const dir=await mkdtemp(join(tmpdir(),'vertex-cleanup-lock-'));
  const path=join(dir,'vertex.db');let holder;
  try{
    const db=new DatabaseSync(path);
    db.exec("PRAGMA journal_mode=WAL; CREATE TABLE urls(id INTEGER PRIMARY KEY,url TEXT,status TEXT,error TEXT,updated_at TEXT); INSERT INTO urls VALUES(1,'https://en.wikipedia.org/?action=edit','pending',NULL,NULL);");db.close();
    const source="import {DatabaseSync} from 'node:sqlite';import {parentPort,workerData} from 'node:worker_threads';const db=new DatabaseSync(workerData);db.exec('BEGIN IMMEDIATE');parentPort.postMessage('locked');setTimeout(()=>{db.exec('COMMIT');db.close();},14500);";
    holder=new Worker(new URL('data:text/javascript,'+encodeURIComponent(source)),{workerData:path});
    await once(holder,'message');
    const result=await cleanQueue(path,{deadline:new Date(Date.now()+30000)});
    assert.equal(result.status,'completed');assert.equal(result.excluded,1);assert.equal(result.totalExcluded,1);
  }finally{if(holder)await holder.terminate();await rm(dir,{recursive:true,force:true});}
});
