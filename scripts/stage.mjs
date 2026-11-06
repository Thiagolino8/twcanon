import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
export const targets = JSON.parse(await fs.readFile(path.join(root, 'targets.json'), 'utf8'));
export const assertBinary = (bytes, target) => {
  let valid = false;
  if (target.os === 'linux' && bytes.length >= 20 && bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    valid = bytes[4] === 2 && bytes[5] === 1 && bytes.readUInt16LE(18) === (target.cpu === 'x64' ? 62 : 183);
  } else if (target.os === 'darwin' && bytes.length >= 8 && bytes.readUInt32LE(0) === 0xfeedfacf) {
    valid = bytes.readUInt32LE(4) === (target.cpu === 'x64' ? 0x1000007 : 0x100000c);
  } else if (target.os === 'win32' && bytes.length >= 64 && bytes.toString('ascii', 0, 2) === 'MZ') {
    const offset = bytes.readUInt32LE(60);
    valid = offset <= bytes.length - 6 && bytes.toString('ascii', offset, offset + 4) === 'PE\0\0' && bytes.readUInt16LE(offset + 4) === (target.cpu === 'x64' ? 0x8664 : 0xaa64);
  }
  if (!valid) throw new Error(`Formato de binário incompatível com ${target.triple}.`);
};

export const stage = async (triple, executable) => {
  const target = targets.find(entry => entry.triple === triple);
  if (!target || !executable) throw new Error('Uso: node scripts/stage.mjs <target-triple> <binário>');
  const manifest = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8'));
  const bytes = await fs.readFile(executable);
  assertBinary(bytes, target);
  const directory = path.join(root, 'dist', target.id);
  await fs.rm(directory, { recursive: true, force: true });
  await fs.mkdir(path.join(directory, 'bin'), { recursive: true });
  await fs.writeFile(path.join(directory, 'bin/twcanon.exe'), bytes, { mode: 0o755 });
  const nativeManifest = {
    name: `twcanon-${target.id}`, version: manifest.version, license: manifest.license,
    description: `Binário nativo de twcanon para ${target.id}.`,
    repository: manifest.repository, homepage: manifest.homepage, bugs: manifest.bugs,
    publishConfig: manifest.publishConfig,
    os: [target.os], cpu: [target.cpu], ...(target.libc ? { libc: [target.libc] } : {}),
    files: ['bin', 'licenses', 'LICENSE', 'README.md'],
    native: { target: triple, sha256: createHash('sha256').update(bytes).digest('hex') },
  };
  await fs.writeFile(path.join(directory, 'package.json'), JSON.stringify(nativeManifest, null, 2) + '\n');
  await fs.copyFile(path.join(root, 'LICENSE'), path.join(directory, 'LICENSE'));
  await fs.writeFile(path.join(directory, 'README.md'), `# ${nativeManifest.name}\n\nBinário de plataforma para [twcanon](https://www.npmjs.com/package/twcanon) ${manifest.version}. Instale o pacote principal para usar a CLI.\n\nAlvo: \`${triple}\`. Licenças dos componentes em \`licenses/\`.\n`);
  await fs.cp(path.join(root, 'licenses'), path.join(directory, 'licenses'), { recursive: true });
  return directory;
};

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.length !== 4) throw new Error('Uso: node scripts/stage.mjs <target-triple> <binário>');
  console.log(await stage(...process.argv.slice(2)));
}
