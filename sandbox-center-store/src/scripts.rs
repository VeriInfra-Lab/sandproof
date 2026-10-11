// No tonumber(revision): Redis Lua numbers cannot exactly represent all u64.
// All type/format/revision/size checks precede the only write (one HSET).
// No EXPIRE, secondary authoritative index, or post-write state mutation.
pub(crate) const READ: &str = r"
local kind = redis.call('TYPE', KEYS[1]).ok
if kind == 'none' then return {1} end
if kind ~= 'hash' or redis.call('HLEN', KEYS[1]) ~= 4 then return {3} end
if redis.call('PTTL', KEYS[1]) ~= -1 then return {3} end
local deployment = redis.call('HGET', KEYS[1], 'deployment')
local format = redis.call('HGET', KEYS[1], 'format')
local revision = redis.call('HGET', KEYS[1], 'revision')
if deployment ~= ARGV[1] or format ~= ARGV[2] or not revision then return {3} end
if redis.call('HSTRLEN', KEYS[1], 'payload') > tonumber(ARGV[3]) then return {3} end
local payload = redis.call('HGET', KEYS[1], 'payload')
if not payload or string.len(payload) == 0 then return {3} end
return {0, revision, payload}
";

pub(crate) const CAS: &str = r"
local kind = redis.call('TYPE', KEYS[1]).ok
if string.len(ARGV[5]) == 0 or string.len(ARGV[5]) > tonumber(ARGV[6]) then return 3 end
if ARGV[3] == '' then
  if kind ~= 'none' then return 2 end
else
  if kind == 'none' then return 1 end
  if kind ~= 'hash' or redis.call('HLEN', KEYS[1]) ~= 4 then return 3 end
  if redis.call('HGET', KEYS[1], 'deployment') ~= ARGV[1] then return 3 end
  if redis.call('HGET', KEYS[1], 'format') ~= ARGV[2] then return 3 end
  if redis.call('HGET', KEYS[1], 'revision') ~= ARGV[3] then return 4 end
  if redis.call('HSTRLEN', KEYS[1], 'payload') > tonumber(ARGV[6]) then return 3 end
  if redis.call('HGET', KEYS[1], 'payload') ~= ARGV[7] then return 3 end
  if redis.call('PTTL', KEYS[1]) ~= -1 then return 3 end
end
redis.call('HSET', KEYS[1], 'deployment', ARGV[1], 'format', ARGV[2],
  'revision', ARGV[4], 'payload', ARGV[5])
return 0
";
