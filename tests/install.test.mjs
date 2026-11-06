import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import path from 'node:path';
import os from 'node:os';
import { createHash } from 'node:crypto';
import { test } from 'node:test';
import { spawnSync } from 'node:child_process';
import { install, selectTarget } from '../install.mjs';

test('selects all supported platforms and rejects unknown architectures', () => {
  for (const cpu of ['x64', 'arm64']) {
    assert.equal(selectTarget('linux', cpu, '2.35').id, `linux-${cpu}-gnu`);
    assert.equal(selectTarget('linux', cpu).id, `linux-${cpu}-musl`);
    assert.equal(selectTarget('darwin', cpu).id, `darwin-${cpu}`);
    assert.equal(selectTarget('win32', cpu).id, `windows-${cpu}`);
  }
  assert.throws(() => selectTarget('linux', 'ia32', '2.35'), /Plataforma sem binário/);
  assert.throws(() => selectTarget('freebsd', 'x64'), /Plataforma sem binário/);
});

test('verifies package identity and content before replacing the entry point', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'tw-npm-install-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const report = process.platform === 'linux' ? process.report.getReport() : undefined;
  const target = selectTarget(process.platform, process.arch, report?.header.glibcVersionRuntime);
  const name = `twcanon-${target.id}`;
  const destination = path.join(root, 'bin/twcanon.exe');
  await fs.mkdir(path.dirname(destination), { recursive: true });
  await fs.writeFile(destination, 'placeholder');
  const cached = path.join(root, 'cached-placeholder');
  await fs.link(destination, cached);
  const otherProject = path.join(root, 'other-project-placeholder');
  await fs.link(destination, otherProject);
  const binLink = path.join(root, 'linked-command');
  await fs.symlink(destination, binLink);
  await fs.writeFile(path.join(root, 'package.json'), JSON.stringify({ version: '0.1.0' }));
  await assert.rejects(install(root), /ausente/);
  const directory = path.join(root, 'node_modules', name);
  await fs.mkdir(path.join(directory, 'bin'), { recursive: true });
  const bytes = Buffer.from('native fixture');
  const manifest = { name, version: '0.1.0', native: { target: target.triple, sha256: createHash('sha256').update(bytes).digest('hex') } };
  const writeManifest = () => fs.writeFile(path.join(directory, 'package.json'), JSON.stringify(manifest));
  await fs.writeFile(path.join(directory, 'bin/twcanon.exe'), 'corrupt');
  await writeManifest();
  await assert.rejects(install(root), /Hash do binário inválido/);
  assert.equal(await fs.readFile(destination, 'utf8'), 'placeholder');
  await fs.writeFile(path.join(directory, 'bin/twcanon.exe'), bytes);
  manifest.version = '0.2.0';
  await writeManifest();
  await assert.rejects(install(root), /Metadados incompatíveis/);
  manifest.version = '0.1.0';
  manifest.native.target = 'wrong-target';
  await writeManifest();
  await assert.rejects(install(root), /Metadados incompatíveis/);
  manifest.native.target = target.triple;
  await writeManifest();
  await install(root);
  assert.deepEqual(await fs.readFile(destination), bytes);
  assert.deepEqual(await fs.readFile(binLink), bytes);
  assert.equal(await fs.readFile(cached, 'utf8'), 'placeholder');
  assert.equal(await fs.readFile(otherProject, 'utf8'), 'placeholder');
  if (process.platform !== 'win32') assert.ok((await fs.stat(destination)).mode & 0o111);
  assert.deepEqual(await fs.readdir(path.dirname(destination)), ['twcanon.exe']);
  await fs.copyFile(new URL('../install.mjs', import.meta.url), path.join(root, 'install.mjs'));
  await fs.copyFile(new URL('../targets.json', import.meta.url), path.join(root, 'targets.json'));
  await fs.writeFile(destination, 'placeholder');
  const alias = path.join(root, 'package-link');
  await fs.symlink(root, alias, process.platform === 'win32' ? 'junction' : 'dir');
  const child = spawnSync(process.execPath, [path.join(alias, 'install.mjs')], { encoding: 'utf8' });
  assert.equal(child.status, 0, child.stderr);
  assert.deepEqual(await fs.readFile(binLink), bytes);
  assert.equal(await fs.readFile(cached, 'utf8'), 'placeholder');
  assert.equal(await fs.readFile(otherProject, 'utf8'), 'placeholder');
});

test('installs through an isolated pnpm package symlink and Windows path aliases', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'tw-pnpm-install-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const report = process.platform === 'linux' ? process.report.getReport() : undefined;
  const target = selectTarget(process.platform, process.arch, report?.header.glibcVersionRuntime);
  const name = `twcanon-${target.id}`;
  const store = path.join(root, 'node_modules/.pnpm/twcanon@0.1.0/node_modules');
  const directory = path.join(store, 'twcanon');
  const native = path.join(store, name);
  const destination = path.join(directory, 'bin/twcanon.exe');
  await fs.mkdir(path.dirname(destination), { recursive: true });
  await fs.mkdir(path.join(native, 'bin'), { recursive: true });
  const bytes = Buffer.from('isolated native fixture');
  await fs.writeFile(path.join(directory, 'package.json'), JSON.stringify({ type: 'module', version: '0.1.0' }));
  await fs.writeFile(path.join(native, 'package.json'), JSON.stringify({
    name, version: '0.1.0', native: { target: target.triple, sha256: createHash('sha256').update(bytes).digest('hex') },
  }));
  await fs.writeFile(path.join(native, 'bin/twcanon.exe'), bytes);
  await fs.copyFile(new URL('../install.mjs', import.meta.url), path.join(directory, 'install.mjs'));
  await fs.copyFile(new URL('../targets.json', import.meta.url), path.join(directory, 'targets.json'));
  const alias = path.join(root, 'node_modules/twcanon');
  await fs.symlink(directory, alias, process.platform === 'win32' ? 'junction' : 'dir');
  await assert.rejects(fs.stat(path.join(root, 'node_modules', name)), { code: 'ENOENT' });
  const paths = [path.join(alias, 'install.mjs')];
  if (process.platform === 'win32') {
    paths.push(paths[0].replace(/^[A-Z]:/, drive => drive.toLowerCase()));
    const short = spawnSync('cmd.exe', ['/d', '/c', 'for %I in (.) do @echo %~fsI'], { cwd: directory, encoding: 'utf8' });
    assert.equal(short.status, 0, short.stderr);
    paths.push(path.join(short.stdout.trim(), 'install.mjs'));
  }
  for (const script of paths) {
    await fs.writeFile(destination, 'placeholder');
    const child = spawnSync(process.execPath, [script], { encoding: 'utf8' });
    assert.equal(child.status, 0, child.stderr);
    assert.deepEqual(await fs.readFile(destination), bytes);
  }
  await fs.writeFile(destination, 'placeholder');
  await install(alias);
  assert.deepEqual(await fs.readFile(destination), bytes);
});
