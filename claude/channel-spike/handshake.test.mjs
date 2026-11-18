// Fake "Claude Code": speaks raw MCP JSON-RPC to channel.mjs and asserts that a
// message pushed to the socket comes out as notifications/claude/channel.
import { spawn } from 'node:child_process';
import assert from 'node:assert/strict';

const id = `test-${process.pid}`;
const child = spawn('node', ['channel.mjs'], {
  env: { ...process.env, TELEX_CHANNEL_ID: id },
  stdio: ['pipe', 'pipe', 'inherit'],
});
const events = []; const responses = new Map();
let buf = '';
child.stdout.on('data', (d) => {
  buf += d;
  let nl;
  while ((nl = buf.indexOf('\n')) >= 0) {
    const m = JSON.parse(buf.slice(0, nl)); buf = buf.slice(nl + 1);
    if (m.id !== undefined) responses.get(m.id)?.(m); else events.push(m);
  }
});
const send = (o) => child.stdin.write(JSON.stringify(o) + '\n');
const call = (idn, method, params) => new Promise((res) => { responses.set(idn, res); send({ jsonrpc: '2.0', id: idn, method, params }); });

const init = await call(1, 'initialize', {
  protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 'fake-claude', version: '0' },
});
assert.deepEqual(init.result.capabilities.experimental['claude/channel'], {}, 'declares claude/channel');
assert.match(init.result.instructions, /untrusted/);
send({ jsonrpc: '2.0', method: 'notifications/initialized' });

await new Promise((r) => setTimeout(r, 300));
const { execFileSync } = await import('node:child_process');
const out = execFileSync('node', ['push.mjs', '--to', id, '--id', 'm1', '--from', 'a/b', '--subject', 'hi', 'ping body'], { encoding: 'utf8' });
assert.match(out, /"ok":true/);
await new Promise((r) => setTimeout(r, 300));

const ev = events.find((e) => e.method === 'notifications/claude/channel');
assert.ok(ev, 'channel notification emitted');
assert.equal(ev.params.content, 'ping body');
assert.deepEqual(ev.params.meta, { id: 'm1', from: 'a/b', subject: 'hi', attention: 'normal' });
console.log('PASS: capability declared, push -> notifications/claude/channel');
child.kill('SIGTERM');
