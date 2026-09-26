// 登録と失効処理をRedis内で確定し、複数のVercelインスタンス間で共有する。
export const registrationScript = `
local now = tonumber(redis.call('TIME')[1])
local entries = redis.call('HGETALL', KEYS[1])
for index = 1, #entries, 2 do
  local peer = cjson.decode(entries[index + 1])
  if peer.expires_at <= now then redis.call('HDEL', KEYS[1], entries[index]) end
end
local previous = redis.call('HGET', KEYS[1], ARGV[1])
local peer = cjson.decode(ARGV[2])
if previous then
  if cjson.decode(previous).certificate_sha256 ~= peer.certificate_sha256 then
    return redis.error_reply('IDENTITY_CONFLICT')
  end
elseif redis.call('HLEN', KEYS[1]) >= tonumber(ARGV[4]) then
  return redis.error_reply('ROOM_FULL')
end
peer.expires_at = now + tonumber(ARGV[3])
redis.call('HSET', KEYS[1], ARGV[1], cjson.encode(peer))
redis.call('EXPIRE', KEYS[1], ARGV[3])
return redis.call('HVALS', KEYS[1])
`;
