import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
const port=Number(process.argv[2]||3110);
if(!Number.isInteger(port)||port<1024||port>65535)throw new Error('Port invalide');
const root=new URL('../apps/landing/public/',import.meta.url);
const files=new Map([
  ['/','index.html'],['/presentation','index.html'],['/index.html','index.html'],
  ['/site.css','site.css'],['/premium.css','premium.css'],['/site.js','site.js'],['/icons.svg','icons.svg'],
  ['/vendor/motion-mini.js','vendor/motion-mini.js'],
  ['/vertex-chrome.webp','vertex-chrome.webp'],['/search-view.png','search-view.png'],
  ['/reader-view.png','reader-view.png'],['/crawler-view.png','crawler-view.png'],
  ['/THIRD-PARTY-NOTICES.txt','THIRD-PARTY-NOTICES.txt'],
]);
const mime={html:'text/html',css:'text/css',js:'text/javascript',svg:'image/svg+xml',webp:'image/webp',png:'image/png',txt:'text/plain'};
createServer(async(req,res)=>{
  const hosts=[`127.0.0.1:${port}`,`localhost:${port}`];
  if(!hosts.includes(req.headers.host?.toLowerCase())||(req.headers.origin&&!hosts.some(host=>req.headers.origin===`http://${host}`))){res.writeHead(403);res.end();return;}
  if(!['GET','HEAD'].includes(req.method)){res.writeHead(405,{Allow:'GET, HEAD'});res.end();return;}
  res.setHeader('X-Content-Type-Options','nosniff');
  res.setHeader('Referrer-Policy','no-referrer');
  res.setHeader('Content-Security-Policy',"default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'");
  let path;
  try{path=new URL(req.url,`http://127.0.0.1:${port}`).pathname;}
  catch{res.writeHead(400);res.end();return;}
  if(path==='/api/release'||path==='/downloads/windows'){res.writeHead(path==='/api/release'?200:503,{'Content-Type':'application/json','Cache-Control':'no-store'});res.end(req.method==='HEAD'?undefined:JSON.stringify({available:false}));return;}
  if(path==='/search'){res.writeHead(302,{Location:'http://127.0.0.1:3108/'});res.end();return;}
  const file=files.get(path);
  if(!file){res.writeHead(404);res.end();return;}
  try{const content=await readFile(fileURLToPath(new URL(file,root)));res.writeHead(200,{'Content-Type':mime[file.split('.').at(-1)],'Content-Length':content.length,'Cache-Control':'no-store'});res.end(req.method==='HEAD'?undefined:content);}
  catch{res.writeHead(404);res.end();}
}).listen(port,'127.0.0.1',()=>console.log(`Vertex site preview: http://127.0.0.1:${port}`));
