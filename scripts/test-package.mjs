import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { selectTarget } from '../install.mjs';
import { npm } from './npm.mjs';
import { fileURLToPath } from 'node:url';

const directory = path.resolve(process.argv[2] ?? 'artifacts');
const manifest = JSON.parse(await fs.readFile(fileURLToPath(new URL('../package.json', import.meta.url)), 'utf8'));
const report = process.platform === 'linux' ? process.report.getReport() : undefined;
const target = selectTarget(process.platform, process.arch, report?.header.glibcVersionRuntime);
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'twcanon-package-'));
const files = await fs.readdir(directory);
const find = expression => {
  const matches = files.filter(name => expression.test(name));
  if (matches.length !== 1) throw new Error(`Esperado um artefato: ${expression}`);
  return path.join(directory, matches[0]);
};
try {
  await fs.writeFile(path.join(root, 'package.json'), '{"private":true}');
  const native = find(new RegExp(`^twcanon-${target.id}-[0-9].*\\.tgz$`));
  const main = find(/^twcanon-[0-9].*\.tgz$/);
  const install = npm(['install', '--no-audit', '--no-fund', native, main], {
    cwd: root, encoding: 'utf8',
  });
  if (install.error) throw install.error;
  assert.equal(install.status, 0, install.stderr);
  const binary = path.join(root, 'node_modules/twcanon/bin/twcanon.exe');
  const command = path.join(root, 'node_modules/.bin', process.platform === 'win32' ? 'twcanon.cmd' : 'twcanon');
  const shim = spawnSync(command, ['--version'], { cwd: root, encoding: 'utf8', shell: process.platform === 'win32' });
  assert.equal(shim.status, 0, shim.stderr);
  assert.equal(shim.stdout.trim(), manifest.version);
  const run = args => spawnSync(binary, args, { cwd: root, encoding: 'utf8', env: { ...process.env, PATH: '', CI: '1' } });
  const version = run(['--version']);
  assert.equal(version.status, 0, version.stderr);
  assert.equal(version.stdout.trim(), manifest.version);
  await fs.mkdir(path.join(root, 'src'));
  await fs.writeFile(path.join(root, 'src/index.css'), '@tailwind utilities;');
  const source = path.join(root, 'src/a.tsx');
  await fs.writeFile(source, 'cn("flex-grow");');
  const check = run(['--check']);
  assert.equal(check.status, 1, check.stderr);
  const fix = run([]);
  assert.equal(fix.status, 0, fix.stderr);
  assert.equal(await fs.readFile(source, 'utf8'), 'cn("grow");');
  const clean = run(['--check']);
  assert.equal(clean.status, 0, clean.stderr);
  await fs.writeFile(source, 'cn("flex-grow");');
  const targetedCheck = run(['--check', source]);
  assert.equal(targetedCheck.status, 1, targetedCheck.stderr);
  const targetedFix = run([source]);
  assert.equal(targetedFix.status, 0, targetedFix.stderr);
  assert.equal(await fs.readFile(source, 'utf8'), 'cn("grow");');
  console.log(`Instalação e execução sem Node no PATH: ${target.id}, OK.`);
} finally {
  await fs.rm(root, { recursive: true, force: true });
}
