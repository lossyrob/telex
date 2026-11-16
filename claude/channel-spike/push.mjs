#!/usr/bin/env node
// Stand-in for `telex claude push`: send one message to a channel server's socket.
//   node push.mjs [--to <id>] [--from addr] [--subject s] [--id msgid] <body...>
// With no --to, uses the most recently created socket.

import net from 'node:net';
import fs from 'node:fs';
import { SOCK_DIR, sockPath } from './paths.mjs';

const args = process.argv.slice(2);
const opt = {};
const rest = [];
for (let i = 0; i < args.length; i++) {
  if (args[i].startsWith('--')) opt[args[i].slice(2)] = args[++i];
  else rest.push(args[i]);
}

let target;
if (opt.to) target = sockPath(opt.to);
else {
  const socks = fs.readdirSync(SOCK_DIR).filter((f) => f.endsWith('.sock'))
    .map((f) => ({ f, t: fs.statSync(`${SOCK_DIR}/${f}`).mtimeMs }))
    .sort((a, b) => b.t - a.t);
  if (!socks.length) { console.error('no channel sockets found'); process.exit(2); }
  target = `${SOCK_DIR}/${socks[0].f}`;
}

const msg = {
  id: opt.id ?? `spike-${Date.now()}`,
  from: opt.from ?? 'spike/push',
  subject: opt.subject ?? 'test',
  attention: 'normal',
  body: rest.join(' ') || 'hello from push.mjs',
};

const c = net.createConnection(target, () => c.write(JSON.stringify(msg) + '\n'));
c.setEncoding('utf8');
c.on('data', (d) => { console.log(d.trim()); c.end(); });
c.on('error', (e) => { console.error('push failed:', e.message); process.exit(1); });
