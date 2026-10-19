import { readdir, readFile, stat, mkdir, writeFile } from 'node:fs/promises';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const excluded = new Set(['.git', 'node_modules', 'target', 'database', 'temp', 'test-results', 'playwright-report', '.vercel', '.wrangler', 'gen']);
const emptyFiles = [], emptyDirectories = [], sensitiveFiles = [], obsoletePaths = [];
let files = 0;
async function visit(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  if (!entries.length) emptyDirectories.push(relative(root, directory));
  for (const entry of entries) {
    if (excluded.has(entry.name) || entry.isSymbolicLink()) continue;
    if (entry.name.startsWith('.env') && entry.name !== '.env.example') continue;
    const path = join(directory, entry.name), name = relative(root, path).replaceAll('\\', '/');
    if (entry.isDirectory()) { await visit(path); continue; }
    if (!entry.isFile()) continue;
    files++;
    const size = (await stat(path)).size;
    if (!size && entry.name !== '.gitkeep') emptyFiles.push(name);
    if (!/\.(?:rs|js|mjs|json|toml|yml|yaml|md|html|ps1|bat|txt)$/.test(name) || size > 2_000_000) continue;
    const text = await readFile(path, 'utf8');
    if (/(?:gh[pousr]_[A-Za-z0-9]{25,}|AKIA[A-Z0-9]{16}|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----)/.test(text)) sensitiveFiles.push(name);
    if (/\.(?:bat|ps1)$/.test(name) && /[A-Za-z]:\\Users\\Administrateur\\Vertex/.test(text)) obsoletePaths.push(name);
  }
}
await visit(root);
const report = { at: new Date().toISOString(), files, emptyFiles, emptyDirectories, sensitiveFiles, obsoletePaths,
  limits: 'Source inventory only; ignored runtime/build directories are excluded. Pattern scanning is not a complete secret or vulnerability audit.' };
await mkdir(join(root, 'test-results'), { recursive: true });
await writeFile(join(root, 'test-results/project-audit.json'), JSON.stringify(report, null, 2));
console.log(JSON.stringify(report, null, 2));
if (emptyFiles.length || sensitiveFiles.length || obsoletePaths.length) process.exitCode = 1;
