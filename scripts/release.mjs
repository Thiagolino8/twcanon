import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { assertBinary, targets } from './stage.mjs';
import { npm } from './npm.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const manifest = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8'));
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const readMember = (archive, member) => {
  const child = spawnSync('tar', ['-xOf', archive, `package/${member}`], { maxBuffer: 64 * 1024 * 1024 });
  if (child.error) throw child.error;
  if (child.status !== 0) throw new Error(`Arquivo ausente no pacote ${path.basename(archive)}: ${member}`);
  return child.stdout;
};

export const validateManifest = (pkg, target) => {
  const name = target ? `twcanon-${target.id}` : 'twcanon';
  if (pkg.name !== name || pkg.version !== manifest.version || pkg.private || pkg.license !== 'MIT') throw new Error(`Identidade inválida: ${name}`);
  if (pkg.repository?.url !== manifest.repository.url || pkg.publishConfig?.access !== 'public' || pkg.publishConfig?.registry !== 'https://registry.npmjs.org/') throw new Error(`Publicação inválida: ${name}`);
  if (target) {
    if (JSON.stringify(pkg.os) !== JSON.stringify([target.os]) || JSON.stringify(pkg.cpu) !== JSON.stringify([target.cpu]) || JSON.stringify(pkg.libc) !== JSON.stringify(target.libc ? [target.libc] : undefined) || pkg.native?.target !== target.triple) throw new Error(`Plataforma inválida: ${name}`);
  } else {
    const expected = Object.fromEntries(targets.map(entry => [`twcanon-${entry.id}`, manifest.version]));
    if (JSON.stringify(pkg.optionalDependencies) !== JSON.stringify(expected) || pkg.bin?.twcanon !== 'bin/twcanon.exe' || pkg.scripts?.postinstall !== 'node install.mjs') throw new Error('Contrato do pacote principal inválido.');
  }
};

export const validateRelease = async directory => {
  const expected = [...targets.map(target => ({ target, name: `twcanon-${target.id}` })), { name: 'twcanon' }];
  const actual = (await fs.readdir(directory)).filter(name => name.endsWith('.tgz')).sort();
  const filenames = expected.map(entry => `${entry.name}-${manifest.version}.tgz`).sort();
  if (JSON.stringify(actual) !== JSON.stringify(filenames)) throw new Error('Artefatos incompletos ou inesperados; são necessários o pacote principal e os oito binários.');
  const packages = [];
  for (const entry of expected) {
    const archive = path.join(directory, `${entry.name}-${manifest.version}.tgz`);
    const pkg = JSON.parse(readMember(archive, 'package.json'));
    validateManifest(pkg, entry.target);
    for (const member of ['LICENSE', 'README.md', 'licenses/index.json']) readMember(archive, member);
    if (entry.target) {
      const bytes = readMember(archive, 'bin/twcanon.exe');
      assertBinary(bytes, entry.target);
      if (sha256(bytes) !== pkg.native.sha256) throw new Error(`Hash do binário inválido: ${entry.name}`);
    } else {
      for (const member of ['install.mjs', 'targets.json', 'bin/twcanon.exe']) {
        if (!readMember(archive, member).equals(await fs.readFile(path.join(root, member)))) throw new Error(`Conteúdo divergente no pacote principal: ${member}`);
      }
    }
    const bytes = await fs.readFile(archive);
    packages.push({ name: entry.name, archive, sha256: sha256(bytes), integrity: `sha512-${createHash('sha512').update(bytes).digest('base64')}` });
  }
  return packages;
};

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [directory = 'artifacts', flag, ...extra] = process.argv.slice(2);
  if ((flag && flag !== '--publish') || extra.length) throw new Error('Uso: node scripts/release.mjs [artefatos] [--publish]');
  const packages = await validateRelease(path.resolve(directory));
  if (flag === '--publish') {
    if (process.env.GITHUB_REPOSITORY !== 'Thiagolino8/twcanon' || process.env.GITHUB_REF !== `refs/tags/v${manifest.version}` || process.env.GITHUB_ACTIONS !== 'true') throw new Error('Publicação permitida somente no workflow do repositório e na tag da versão.');
    for (const pkg of packages) {
      const query = npm(['view', `${pkg.name}@${manifest.version}`, 'dist.integrity', '--json', '--registry=https://registry.npmjs.org/'], { encoding: 'utf8' });
      if (query.error) throw query.error;
      if (query.status === 0) {
        if (JSON.parse(query.stdout) !== pkg.integrity) throw new Error(`Versão já publicada com conteúdo diferente: ${pkg.name}`);
        console.log(`Já publicado e verificado: ${pkg.name}`);
        continue;
      }
      if (!query.stderr.includes('E404')) throw new Error(query.stderr);
      const child = npm(['publish', pkg.archive, '--provenance', '--access', 'public', '--registry=https://registry.npmjs.org/'], { stdio: 'inherit' });
      if (child.error) throw child.error;
      if (child.status !== 0) throw new Error(`Publicação falhou: ${pkg.name}`);
    }
  }
  console.log(JSON.stringify(packages.map(({ name, sha256 }) => ({ name, sha256 })), null, 2));
}
