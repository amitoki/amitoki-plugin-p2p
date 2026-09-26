import assert from 'node:assert/strict';
import { randomBytes, randomUUID } from 'node:crypto';
import { test, before, after } from 'node:test';
import { createClient } from 'redis';
import { createRendezvous } from '../server/rendezvous.mjs';

const token = randomBytes(32).toString('hex');
const room = randomUUID();
const certificate = 'a'.repeat(64);
const registration = { room, node_id: 'a', address: '127.0.0.1:7443', certificate_sha256: certificate };
function request(body, authorization = `Bearer ${token}`) {
  return new Request('http://localhost/api/peers', { method: 'POST', headers: { authorization }, body: JSON.stringify(body) });
}
const stub = createRendezvous({ token, redis: { eval() { throw new Error('unexpected Redis call'); } } });
test('認証なしの登録を拒否する', async () => assert.equal((await stub(request(registration, ''))).status, 401));
test('パケット本体や未知の項目を受け付けない', async () => assert.equal((await stub(request({ ...registration, frames: [] }))).status, 400));
test('不正なアドレスと巨大な要求を拒否する', async () => {
  assert.equal((await stub(request({ ...registration, address: 'attacker/path' }))).status, 400);
  assert.equal((await stub(request({ ...registration, room: 'x'.repeat(5000) }))).status, 400);
});

const url = process.env.AMITOKI_TEST_REDIS_URL;
let client;
before(async () => { if (url) { client = createClient({ url }); await client.connect(); } });
after(async () => { if (client) await client.quit(); });
test('Redisを共有する複数インスタンスで登録・部屋分離・ID衝突拒否が働く', { skip: !url }, async () => {
  const redis = { eval: (script, keys, arguments_) => client.eval(script, { keys, arguments: arguments_ }) };
  const first = createRendezvous({ token, redis });
  const second = createRendezvous({ token, redis });
  assert.equal((await first(request(registration))).status, 200);
  const response = await second(request({ ...registration, node_id: 'b', address: '127.0.0.1:7444' }));
  assert.equal(response.status, 200);
  assert.deepEqual((await response.json()).peers.map((peer) => peer.node_id).sort(), ['a', 'b']);
  const other = await first(request({ ...registration, room: `${room}-other` }));
  assert.equal((await other.json()).peers.length, 1);
  assert.equal((await first(request({ ...registration, certificate_sha256: 'b'.repeat(64) }))).status, 409);
});
