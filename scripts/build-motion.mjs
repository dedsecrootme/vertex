import {build} from 'esbuild';
import {readFile,writeFile} from 'node:fs/promises';

await build({
  entryPoints:['motion/mini'],bundle:true,minify:true,format:'esm',target:'es2022',
  outfile:'apps/landing/public/vendor/motion-mini.js',legalComments:'eof',
});
const icons=await readFile('apps/web/THIRD-PARTY-NOTICES.txt','utf8');
const notices=[icons];
for(const name of ['motion','motion-dom','motion-utils']){
  const license=await readFile(`node_modules/${name}/LICENSE.md`,'utf8');
  const {version}=JSON.parse(await readFile(`node_modules/${name}/package.json`,'utf8'));
  notices.push(`${name} ${version}\n\n${license}`);
}
await writeFile('apps/landing/public/THIRD-PARTY-NOTICES.txt',notices.join('\n---\n\n'));
