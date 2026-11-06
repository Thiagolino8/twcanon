import assert from 'node:assert/strict';
import { test } from 'node:test';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { assertBinary, targets } from '../scripts/stage.mjs';
import { validateManifest, validateRelease } from '../scripts/release.mjs';

test('release rejects absent artifacts before publication', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'twcanon-release-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  await assert.rejects(validateRelease(directory), /Artefatos incompletos/);
});

test('release checks version, platform metadata and binary architecture', async () => {
  const pkg = JSON.parse(await fs.readFile(new URL('../package.json', import.meta.url), 'utf8'));
  validateManifest(pkg);
  assert.throws(() => validateManifest({ ...pkg, version: '9.9.9' }), /Identidade inválida/);
  assert.throws(() => validateManifest({ ...pkg, optionalDependencies: {} }), /Contrato/);
  for (const target of targets) {
    assert.throws(() => assertBinary(Buffer.from('placeholder'), target), /Formato/);
    const native = {
      ...pkg, name: `twcanon-${target.id}`, os: [target.os], cpu: [target.cpu],
      libc: target.libc ? [target.libc] : undefined, native: { target: target.triple },
    };
    validateManifest(native, target);
    assert.throws(() => validateManifest({ ...native, cpu: ['ia32'] }, target), /Plataforma/);
    if (target.os === 'win32') {
      assert.equal(native.name, `twcanon-windows-${target.cpu}`);
      assert.throws(() => validateManifest({ ...native, name: `twcanon-win32-${target.cpu}` }, target), /Identidade/);
      assert.throws(() => validateManifest({ ...native, os: ['windows'] }, target), /Plataforma/);
    }
  }
});
