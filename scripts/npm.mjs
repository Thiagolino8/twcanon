import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';

let command = 'npm';
let prefix = [];
if (process.platform === 'win32') {
  const where = spawnSync('where.exe', ['npm.cmd'], { encoding: 'utf8' });
  if (where.error) throw where.error;
  const cli = (where.stdout ?? '').trim().split(/\r?\n/)
    .map(file => path.join(path.dirname(file), 'node_modules/npm/bin/npm-cli.js'))
    .find(file => existsSync(file));
  if (!cli) throw new Error('npm-cli.js não encontrado junto à instalação do Node.');
  command = process.execPath;
  prefix = [cli];
}

export const npm = (args, options) => spawnSync(command, [...prefix, ...args], options);
