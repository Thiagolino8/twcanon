import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { stage } from './stage.mjs';
import { npm } from './npm.mjs';
import { createHash } from 'node:crypto';

const root = fileURLToPath(new URL('../', import.meta.url));
const build = JSON.parse(await fs.readFile(path.join(root, 'build/build.json'), 'utf8'));
const bytes = await fs.readFile(path.join(root, build.binary));
if (createHash('sha256').update(bytes).digest('hex') !== build.sha256) throw new Error('O executável mudou desde o build registrado.');
const native = await stage(build.target, path.join(root, build.binary));
const artifacts = path.join(root, 'artifacts');
await fs.mkdir(artifacts, { recursive: true });
for (const cwd of [native, root]) {
  const child = npm(['pack', '--ignore-scripts', '--json', '--pack-destination', artifacts], {
    cwd, encoding: 'utf8',
  });
  if (child.error) throw child.error;
  if (child.status !== 0) throw new Error(child.stderr);
  console.log(JSON.parse(child.stdout)[0].filename);
}
