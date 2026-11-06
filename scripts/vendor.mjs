import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';

const vendor = new URL('../native/vendor/', import.meta.url);
const verify = async (base, files) => {
  for (const [name, expected] of Object.entries(files)) {
    const bytes = await fs.readFile(new URL(name, base));
    if (createHash('sha256').update(bytes).digest('hex') !== expected) throw new Error(`Fonte vendorizada inválida: ${name}`);
  }
};
const tailwind = JSON.parse(await fs.readFile(new URL('manifest.json', vendor), 'utf8'));
if (tailwind.version !== '4.3.4') throw new Error('A referência Tailwind deve permanecer em 4.3.4.');
await verify(new URL('tailwindcss/', vendor), tailwind.files);
for (const name of ['unicode', 'v8', 'regress']) {
  const base = new URL(`${name}/`, vendor);
  const manifest = JSON.parse(await fs.readFile(new URL('manifest.json', base), 'utf8'));
  await verify(base, manifest.files);
  if (name === 'regress') await verify(base, { [manifest.archive]: manifest.archive_sha256, [manifest.patch]: manifest.patch_sha256 });
}
console.log('Fontes vendorizadas e patches verificados.');
