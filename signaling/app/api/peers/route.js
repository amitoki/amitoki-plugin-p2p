import { Redis } from '@upstash/redis';
import { createRendezvous } from '../../../server/rendezvous.mjs';

export const runtime = 'nodejs';
export const dynamic = 'force-dynamic';
let register;
export async function POST(request) {
  if (!register) {
    try {
      register = createRendezvous({ redis: Redis.fromEnv(), token: process.env.SIGNALING_TOKEN });
    } catch {
      return Response.json({ error: 'signaling_not_configured' }, { status: 503 });
    }
  }
  return register(request);
}
