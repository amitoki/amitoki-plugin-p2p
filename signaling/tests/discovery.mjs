import { createServer } from 'node:http';
import { randomBytes } from 'node:crypto';
import { once } from 'node:events';
import { spawn } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { watch } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createClient } from 'redis';
import { createRendezvous } from '../server/rendezvous.mjs';

const client = createClient({ url: process.env.STEGRDB_TEST_REDIS_URL });
await client.connect();
const token = randomBytes(32).toString('hex');
const register = createRendezvous({ token, redis: { eval: (script, keys, arguments_) => client.eval(script, { keys, arguments: arguments_ }) } });
const directory = await mkdtemp(join(tmpdir(), 'stegrdb-discovery-'));
const stopFile = join(directory, 'stop');
const server = createServer(async (incoming, outgoing) => {
  const request = new Request(`http://localhost${incoming.url}`, { method: incoming.method, headers: incoming.headers,
    ...(incoming.method === 'POST' ? { body: incoming, duplex: 'half' } : {}) });
  const response = await register(request);
  outgoing.writeHead(response.status, Object.fromEntries(response.headers));
  outgoing.end(await response.text());
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const port = server.address().port;
const watcher = watch(directory, (_event, name) => { if (name === 'stop') { server.closeAllConnections(); server.close(); } });
try {
  const child = spawn('cargo', ['test', '--locked', '--test', 'peers', 'discovery_connects_three_peers', '--', '--ignored', '--nocapture'], {
    cwd: new URL('../../', import.meta.url), stdio: 'inherit',
    env: { ...process.env, STEGRDB_TEST_SIGNALING_URL: `http://127.0.0.1:${port}/api/peers`, STEGRDB_TEST_SIGNALING_TOKEN: token, STEGRDB_TEST_SIGNALING_STOP_FILE: stopFile },
  });
  const [status] = await once(child, 'exit');
  if (status !== 0) process.exitCode = 1;
} finally {
  watcher.close(); server.closeAllConnections(); server.close();
  await client.quit(); await rm(directory, { recursive: true, force: true });
}
