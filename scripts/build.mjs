import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const native = path.join(root, 'native');
const args = process.argv.slice(2);
const pgo = args.includes('--pgo');
const targetIndex = args.indexOf('--target');
const target = targetIndex < 0 ? undefined : args[targetIndex + 1];
const validArgs = pgo ? ['--pgo'] : [];
if (targetIndex >= 0 && target) validArgs.push('--target', target);
if (args.length !== validArgs.length || args.some(value => !validArgs.includes(value))) {
  throw new Error('Uso: node scripts/build.mjs [--pgo] [--target <triple>]');
}
const run = (command, argv, options = {}) => {
  const child = spawnSync(command, argv, { cwd: native, stdio: 'inherit', ...options });
  if (child.error) throw child.error;
  if (child.status !== 0) throw new Error(`${command} terminou com status ${child.status}.`);
};
const capture = (command, argv) => {
  const child = spawnSync(command, argv, { cwd: native, encoding: 'utf8' });
  if (child.error) throw child.error;
  if (child.status !== 0) throw new Error(child.stderr);
  return child.stdout.trim();
};
const base = ['build', '--release', '--locked', '--bin', 'twcanon', ...(target ? ['--target', target] : [])];
const manifest = JSON.parse(await fs.readFile(path.join(root, 'package.json'), 'utf8'));
const metadata = JSON.parse(capture('cargo', ['metadata', '--locked', '--no-deps', '--format-version=1']));
if (metadata.packages.find(pkg => pkg.name === 'tailwind-canonical-native')?.version !== manifest.version) {
  throw new Error('A versão do Cargo deve ser igual à versão npm.');
}
const environment = { ...process.env };
delete environment.RUSTFLAGS;
delete environment.CARGO_ENCODED_RUSTFLAGS;
const host = capture('rustc', ['-vV']).match(/^host: (.+)$/m)?.[1];
if (pgo && process.platform === 'win32' && (target ?? host) === 'aarch64-pc-windows-msvc') {
  // link.exe desalinha os marcadores de perfil no Windows arm64; rust-lld preserva o alinhamento.
  const linker = path.join(capture('rustc', ['--print', 'sysroot']), 'lib/rustlib', host, 'bin/rust-lld.exe');
  await fs.access(linker);
  environment.CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER = linker;
}
const buildDirectory = path.join(native, 'target');
const executable = (...directories) => path.join(...directories, ...(target ? [target] : []), 'release', `twcanon${process.platform === 'win32' ? '.exe' : ''}`);
let profile;
if (pgo) {
  const raw = path.join(root, 'build/pgo/raw');
  await fs.rm(raw, { recursive: true, force: true });
  await fs.mkdir(raw, { recursive: true });
  const generate = path.join(buildDirectory, 'pgo-generate');
  run('cargo', [...base, '--target-dir', generate], { env: { ...environment, CARGO_ENCODED_RUSTFLAGS: `-Cprofile-generate=${raw}` } });
  const training = path.join(root, 'build/pgo/training');
  await fs.mkdir(path.join(training, 'src'), { recursive: true });
  const css = await fs.readFile(path.join(native, 'fixtures/default-theme.css'), 'utf8');
  await fs.writeFile(path.join(training, 'src/index.css'), `${css}\n@tailwind utilities;\n`);
  const values = new Set(['flex-grow', 'flex-shrink-0', 'mt-2 mb-2', 'w-4 h-4', 'text-sm leading-6', 'bg-[#fff]', 'p-[16px]', 'hover:mt-2 hover:mb-2']);
  for (const prefix of ['', 'hover:', 'focus:', 'md:', 'dark:']) {
    for (const utility of ['p', 'px', 'm', 'mt', 'w', 'h', 'gap', 'rounded']) {
      for (const size of ['0', '1', '2', '4', '8', '[16px]', '[1rem]']) values.add(`${prefix}${utility}-${size}`);
    }
    for (const color of ['red-500', 'blue-600', 'white', 'black', '[#fff]', '[var(--brand)]']) {
      values.add(`${prefix}bg-${color} text-${color}/50 border-${color}`);
    }
  }
  await fs.writeFile(path.join(training, 'src/training.tsx'), [...values].map(value => `cn(${JSON.stringify(value)});`).join('\n'));
  for (let pass = 0; pass < 3; pass++) {
    const child = spawnSync(executable(generate), ['--check', '--no-cache'], {
      cwd: training, stdio: 'ignore', env: { ...process.env, CI: '1', LLVM_PROFILE_FILE: path.join(raw, 'cli-%m-%p.profraw') },
    });
    if (child.error) throw child.error;
    if (child.status !== 1) throw new Error(`Treinamento PGO terminou com status ${child.status}; esperado: 1.`);
  }
  const suffix = process.platform === 'win32' ? '.exe' : '';
  const profdata = path.join(capture('rustc', ['--print', 'sysroot']), 'lib/rustlib', host, 'bin', `llvm-profdata${suffix}`);
  const profiles = (await fs.readdir(raw)).filter(name => name.startsWith('cli-') && name.endsWith('.profraw')).map(name => path.join(raw, name));
  if (!profiles.length) throw new Error('Treinamento PGO não gerou perfis.');
  profile = path.join(root, 'build/pgo/training.profdata');
  run(profdata, ['merge', '-o', profile, ...profiles]);
}
run('cargo', base, { env: { ...environment, CARGO_TARGET_DIR: buildDirectory, CARGO_ENCODED_RUSTFLAGS: profile ? [`-Cprofile-use=${profile}`, '-Cllvm-args=-pgo-warn-missing-function'].join('\u001f') : '' } });
const binary = executable(buildDirectory);
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
await fs.mkdir(path.join(root, 'build'), { recursive: true });
await fs.writeFile(path.join(root, 'build/build.json'), JSON.stringify({
  binary: path.relative(root, binary), sha256: sha256(await fs.readFile(binary)),
  toolchain: capture('rustc', ['--version']), target: target ?? host,
  pgo: Boolean(profile), profile_sha256: profile ? sha256(await fs.readFile(profile)) : null,
  training: profile ? 'Synthetic utility/variant/collapse cases, three CLI passes, no persistent cache.' : null,
}, null, 2) + '\n');
console.log(binary);
