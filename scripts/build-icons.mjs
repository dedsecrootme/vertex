import { readFile, writeFile, copyFile } from 'node:fs/promises';
import { DOMParser, XMLSerializer } from '@xmldom/xmldom';

const names = ['search', 'book-open', 'external-link', 'bookmark', 'shield-check', 'download', 'arrow-up-right', 'pause', 'play', 'square', 'x', 'refresh-cw', 'activity', 'menu', 'arrow-down', 'arrow-up', 'arrow-left', 'maximize-2', 'plus', 'minus', 'monitor'];
const parser = new DOMParser();
const serializer = new XMLSerializer();
const symbols = [];
for (const name of names) {
  const source = parser.parseFromString(await readFile(`node_modules/lucide-static/icons/${name}.svg`, 'utf8'), 'image/svg+xml').documentElement;
  const contents = Array.from(source.childNodes).map(node => serializer.serializeToString(node)).join('');
  symbols.push(`<symbol id="${name}" viewBox="0 0 24 24">${contents}</symbol>`);
}
const sprite = `<!-- Lucide icons, ISC license. See THIRD-PARTY-NOTICES.txt. -->\n<svg xmlns="http://www.w3.org/2000/svg">${symbols.join('\n')}</svg>\n`;
await writeFile('apps/web/icons.svg', sprite);
await writeFile('apps/landing/public/icons.svg', sprite);
const license = await readFile('node_modules/lucide-static/LICENSE', 'utf8');
await writeFile('apps/web/THIRD-PARTY-NOTICES.txt', `Lucide icons 1.59.0\n\n${license}`);
await copyFile('apps/web/THIRD-PARTY-NOTICES.txt', 'apps/landing/public/THIRD-PARTY-NOTICES.txt');
// Regenerating icons must also preserve the landing animation library's notice.
await import('./build-motion.mjs');
