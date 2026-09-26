import { createHash, timingSafeEqual } from 'node:crypto';
import { isIP } from 'node:net';
import { registrationScript } from './registry.mjs';

// 接続情報だけを扱い、パケット本体や巨大な要求は受け付けない。
const MAX_REQUEST_BYTES = 4096;
const REGISTRATION_SECONDS = 60;
const MAX_ROOM_PEERS = 33;
const allowedFields = new Set(['room', 'node_id', 'address', 'certificate_sha256']);
const digest = (value) => createHash('sha256').update(value).digest();
const reply = (body, status = 200) => Response.json(body, { status, headers: { 'Cache-Control': 'no-store' } });

function isIdentifier(value) {
  return typeof value === 'string' && value.trim().length > 0 && Buffer.byteLength(value) <= 128 && !/[\u0000-\u001f\u007f]/u.test(value);
}
function isAddress(value) {
  if (typeof value !== 'string') return false;
  const match = value.match(/^(?:\[([^\]]+)\]|([^:]+)):(\d+)$/u);
  return Boolean(match && isIP(match[1] ?? match[2]) && Number(match[3]) >= 1 && Number(match[3]) <= 65535);
}
async function readRegistration(request) {
  if (!request.body) throw new Error('INVALID_REQUEST');
  const reader = request.body.getReader();
  const chunks = [];
  let length = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > MAX_REQUEST_BYTES) throw new Error('INVALID_REQUEST');
      chunks.push(value);
    }
  } finally {
    await reader.cancel();
  }
  const registration = JSON.parse(Buffer.concat(chunks).toString('utf8'));
  if (!registration || typeof registration !== 'object' || Array.isArray(registration) || Object.keys(registration).some((name) => !allowedFields.has(name)) ||
      !isIdentifier(registration.room) || !isIdentifier(registration.node_id) || !isAddress(registration.address) ||
      typeof registration.certificate_sha256 !== 'string' || !/^[a-f0-9]{64}$/u.test(registration.certificate_sha256)) {
    throw new Error('INVALID_REQUEST');
  }
  return registration;
}

export function createRendezvous({ redis, token }) {
  if (typeof token !== 'string' || token.length < 32) throw new Error('SIGNALING_TOKEN must contain at least 32 characters');
  const expected = digest(`Bearer ${token}`);
  return async function register(request) {
    if (!timingSafeEqual(expected, digest(request.headers.get('authorization') ?? ''))) return reply({ error: 'unauthorized' }, 401);
    let registration;
    try { registration = await readRegistration(request); }
    catch { return reply({ error: 'invalid_registration' }, 400); }
    const key = `stegrdb:room:${digest(registration.room).toString('hex')}`;
    try {
      const values = await redis.eval(registrationScript, [key], [registration.node_id, JSON.stringify(registration), String(REGISTRATION_SECONDS), String(MAX_ROOM_PEERS)]);
      const peers = values.map((value) => {
        const { expires_at, ...peer } = typeof value === 'string' ? JSON.parse(value) : value;
        return peer;
      });
      return reply({ peers });
    } catch (error) {
      if (String(error.message).includes('IDENTITY_CONFLICT')) return reply({ error: 'identity_conflict' }, 409);
      if (String(error.message).includes('ROOM_FULL')) return reply({ error: 'room_full' }, 409);
      return reply({ error: 'registry_unavailable' }, 503);
    }
  };
}
