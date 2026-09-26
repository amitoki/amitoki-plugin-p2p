#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
container=$(docker run --detach --rm --publish 127.0.0.1::6379 redis:7-alpine)
trap 'docker stop "$container" >/dev/null' EXIT
port=$(docker port "$container" 6379/tcp | cut -d: -f2)
export AMITOKI_TEST_REDIS_URL="redis://127.0.0.1:$port"
# Redis自身のreadinessで待つ。起動を固定時間のsleepで仮定しない。
for ((attempt=0; attempt<30; attempt++)); do
  if docker exec "$container" redis-cli ping >/dev/null 2>&1; then break; fi
  sleep 1
done
docker exec "$container" redis-cli ping >/dev/null
npm --prefix signaling ci --no-audit
npm --prefix signaling test
node signaling/tests/discovery.mjs
