import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const run = (command, args, cwd = root) => {
  const child = spawnSync(command, args, { cwd, stdio: 'inherit' });
  if (child.error) throw child.error;
  if (child.status !== 0) process.exit(child.status ?? 1);
};
run(process.execPath, ['scripts/fixtures.mjs']);
run(process.execPath, ['scripts/licenses.mjs']);
run(process.execPath, ['scripts/vendor.mjs']);
const native = path.join(root, 'native');
run('cargo', ['fmt', '--all', '--check'], native);
run('cargo', ['test', '--release', '--locked'], native);
run('cargo', ['clippy', '--locked', '--all-targets', '--', '-D', 'warnings'], native);
run(process.execPath, ['--test', 'tests/install.test.mjs', 'tests/release.test.mjs']);
run(process.execPath, ['scripts/build.mjs']);
run(process.platform === 'win32' ? 'python' : 'python3', ['tests/cli.py']);
