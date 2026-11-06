import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';
import { spawnSync } from 'node:child_process';

const root = new URL('../', import.meta.url);
const index = JSON.parse(await fs.readFile(new URL('licenses/index.json', root), 'utf8'));
for (const component of index.components) {
  for (const entry of component.files) {
    const bytes = await fs.readFile(new URL(entry.file, new URL('licenses/', root)));
    if (createHash('sha256').update(bytes).digest('hex') !== entry.sha256) throw new Error(`Aviso de licença inválido: ${entry.file}`);
  }
}
const metadata = spawnSync('cargo', ['metadata', '--locked', '--format-version=1'], { cwd: new URL('native/', root), encoding: 'utf8', maxBuffer: 8 * 1024 * 1024 });
if (metadata.error) throw metadata.error;
if (metadata.status !== 0) throw new Error(metadata.stderr);
for (const pkg of JSON.parse(metadata.stdout).packages) {
  if (pkg.name === 'tailwind-canonical-native') continue;
  if (!index.components.some(component => component.name === pkg.name && component.version === pkg.version && component.files.length)) throw new Error(`Aviso de dependência ausente: ${pkg.name}@${pkg.version}`);
}
console.log('Inventário de licenças e hashes verificados.');
