import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';
import { gunzipSync } from 'node:zlib';

const directory = new URL('../native/fixtures/', import.meta.url);
const manifest = JSON.parse(await fs.readFile(new URL('manifest.json', directory), 'utf8'));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
for (const [name, entry] of Object.entries(manifest)) {
  const compressed = await fs.readFile(new URL(`${name}.gz`, directory));
  if (hash(compressed) !== entry.gzip_sha256) throw new Error(`Fixture comprimido inválido: ${name}`);
  const bytes = gunzipSync(compressed);
  if (bytes.length !== entry.bytes || hash(bytes) !== entry.sha256) throw new Error(`Fixture inválido: ${name}`);
  await fs.writeFile(new URL(name, directory), bytes);
}
console.log(`${Object.keys(manifest).length} fixtures verificados e extraídos.`);
