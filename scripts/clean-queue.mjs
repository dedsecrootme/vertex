import {DatabaseSync} from 'node:sqlite';
import {existsSync,readFileSync,writeFileSync,renameSync,statSync,statfsSync} from 'node:fs';
import {resolve,dirname,join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';

const roots=['wikipedia.org','wikimedia.org','wiktionary.org','wikisource.org','wikibooks.org','wikiquote.org','wikivoyage.org','wikidata.org'];
const actions=new Set(['edit','submit','history','delete','protect','rollback','purge']);
export function exclusionReason(value){
  let url;try{url=new URL(value);}catch{return null;}
  if(!['http:','https:'].includes(url.protocol))return null;
  if(!roots.some(root=>url.hostname===root||url.hostname.endsWith('.'+root)))return null;
  for(const [key,value] of url.searchParams){
    if((key==='redlink'&&value==='1')||((key==='action'||key==='veaction')&&actions.has(value)))return 'Interface Wikimedia hors contenu : edition, historique ou lien rouge';
  }
  return null;
}

export async function cleanQueue(path,{deadline,stopFile,dryRun=false,batchSize=1000,minFreeMiB=0,maxStorageMiB=81920}={}){
  if(!Number.isInteger(batchSize)||batchSize<1||batchSize>2000)throw new Error('Invalid batch size');
  const end=new Date(deadline||Date.now()+9*3600000).getTime();
  if(!Number.isFinite(end)||end<=Date.now())throw new Error('Invalid cleanup deadline');
  path=resolve(path);
  if(!existsSync(path))throw new Error('Existing database required');
  const statePath=join(dirname(path),'night-cleanup.json');
  const previous=existsSync(statePath)?JSON.parse(readFileSync(statePath,'utf8')):{};
  const previousExcluded=previous.totalExcluded??previous.excluded??0;
  const previousScanned=previous.totalScanned??previous.scanned??0;
  const db=new DatabaseSync(path,{readOnly:dryRun});
  db.exec('PRAGMA busy_timeout=500; PRAGMA cache_size=-8192;');
  if(!dryRun)db.exec('PRAGMA synchronous=NORMAL; PRAGMA journal_size_limit=67108864;');
  const upper=db.prepare('SELECT max(id) AS id FROM urls').get().id||0;
  let cursor=dryRun?0:(previous.cursor||0),scanned=0,excluded=0,candidates=0;
  let running=true;
  const halt=()=>{running=false;};process.on('SIGTERM',halt);process.on('SIGINT',halt);
  const save=status=>{
    const state={status,dryRun,cursor,upper,scanned,excluded,candidates,totalExcluded:previousExcluded+excluded,totalScanned:previousScanned+scanned,completedAt:new Date().toISOString(),deadline:new Date(end).toISOString()};
    if(!dryRun){writeFileSync(statePath+'.tmp',JSON.stringify(state,null,2));renameSync(statePath+'.tmp',statePath);}
    return state;
  };
  try{
    const read=db.prepare('SELECT id,url,status FROM urls WHERE id>? AND id<=? ORDER BY id LIMIT ?');
    const update=dryRun?null:db.prepare("UPDATE urls SET status='skipped',error=?,updated_at=CURRENT_TIMESTAMP WHERE id=? AND url=? AND status='pending'");
    while(running&&Date.now()<end&&!(stopFile&&existsSync(stopFile))){
      const space=statfsSync(dirname(path));
      const size=[path,path+'-wal'].reduce((total,file)=>total+(existsSync(file)?statSync(file).size:0),0);
      if(space.bavail*space.bsize<minFreeMiB*1048576||size>=maxStorageMiB*1048576)return save('storage-limit');
      const rows=read.all(cursor,upper,batchSize);
      if(!rows.length)break;
      for(let offset=0;offset<rows.length;offset+=50){
        if(!running||Date.now()>=end||(stopFile&&existsSync(stopFile)))break;
        const slice=rows.slice(offset,offset+50);
        const matches=slice.filter(row=>row.status==='pending'&&exclusionReason(row.url));
        if(matches.length&&!dryRun){
          let committed=false;
          for(let attempt=0;!committed;attempt++){
            if(!running||Date.now()>=end||(stopFile&&existsSync(stopFile)))break;
            let changes=0;
            try{
              db.exec('BEGIN IMMEDIATE');
              for(const row of matches)changes+=update.run(exclusionReason(row.url),row.id,row.url).changes;
              db.exec('COMMIT');excluded+=changes;committed=true;
            }catch(error){
              try{db.exec('ROLLBACK');}catch{}
              if(!/locked|busy/i.test(error.message))throw error;
              if(attempt%12===11)save('waiting-for-writer');
              await delay(Math.min(2000,250+attempt*50));
            }
          }
          if(!committed){save('paused');return;}
        }
        scanned+=slice.length;candidates+=matches.length;cursor=slice.at(-1).id;
        if(matches.length)await delay(125);
      }
      save('running');
      if(scanned%50000<batchSize)console.log(JSON.stringify({cursor,scanned,excluded,candidates}));
      await delay(20);
    }
    const result=save(cursor>=upper?'completed':'paused');
    console.log(JSON.stringify(result));return result;
  }finally{process.removeListener('SIGTERM',halt);process.removeListener('SIGINT',halt);db.close();}
}

if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
  await cleanQueue(process.argv[2]||'database/vertex.db',{deadline:process.argv[3],stopFile:process.argv[4],dryRun:process.argv.includes('--dry-run'),minFreeMiB:51200});
}
