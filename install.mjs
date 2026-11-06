import { createHash, randomUUID } from 'node:crypto';
import { realpathSync } from 'node:fs';
import fs from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const targets = JSON.parse(await fs.readFile(new URL('./targets.json', import.meta.url), 'utf8'));

export const selectTarget = (platform, arch, glibcVersion) => {
  const libc = platform === 'linux' ? (glibcVersion ? 'glibc' : 'musl') : undefined;
  const target = targets.find(entry => entry.os === platform && entry.cpu === arch && entry.libc === libc);
  if (!target) throw new Error(`Plataforma sem binário: ${platform}/${arch}${libc ? `/${libc}` : ''}.`);
  return target;
};

export const install = async (root = here) => {
  root = await fs.realpath(root);
  const report = process.platform === 'linux' ? process.report.getReport() : undefined;
  const target = selectTarget(process.platform, process.arch, report?.header.glibcVersionRuntime);
  const manifest = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8'));
  const packageName = `twcanon-${target.id}`;
  const require = createRequire(path.join(root, 'package.json'));
  let packagePath;
  try {
    packagePath = require.resolve(`${packageName}/package.json`);
  } catch (cause) {
    throw new Error(`Pacote ${packageName}@${manifest.version} ausente. Instale as dependências opcionais e permita o postinstall.`, { cause });
  }
  const native = JSON.parse(await fs.readFile(packagePath, 'utf8'));
  if (native.name !== packageName || native.version !== manifest.version || native.native?.target !== target.triple) {
    throw new Error(`Metadados incompatíveis em ${packageName}.`);
  }
  const bytes = await fs.readFile(path.join(path.dirname(packagePath), 'bin/twcanon.exe'));
  if (createHash('sha256').update(bytes).digest('hex') !== native.native.sha256) {
    throw new Error(`Hash do binário inválido em ${packageName}.`);
  }
  const destination = path.join(root, 'bin/twcanon.exe');
  await fs.mkdir(path.dirname(destination), { recursive: true });
  const temporary = `${destination}.${randomUUID()}.tmp`;
  try {
    // Package managers can share the placeholder inode with their package cache.
    await fs.writeFile(temporary, bytes, { flag: 'wx', mode: 0o755 });
    await fs.chmod(temporary, 0o755);
    await fs.rename(temporary, destination);
  } finally {
    await fs.rm(temporary, { force: true });
  }
};

const invoked = process.argv[1] ? path.resolve(process.argv[1]) : undefined;
// Use the loader's non-native realpath algorithm, including Windows 8.3 paths.
const main = invoked ? realpathSync(invoked) : undefined;
if (main && pathToFileURL(main).href === import.meta.url) {
  try {
    await install();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
