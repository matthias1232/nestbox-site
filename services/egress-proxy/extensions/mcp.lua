-- Recognize MCP on the wire so a tool call is a fact, not an inference.
--
-- MCP is JSON-RPC 2.0 over HTTP with a known method set. A request that is
-- `Content-Type: application/json`, has `jsonrpc: "2.0"`, and a `method` in the
-- MCP family is MCP; anything else is "some JSON" and is left alone. For
-- `tools/call` the tool name is `params.name`, and `Mcp-Session-Id` names the
-- session (the request header, or the one the server assigns in its response).
--
-- The result is written into dynamic metadata (`nestbox:*`) where the access
-- log and the decider can read it, and the body is handed on unchanged. This
-- filter never rewrites a request: a proxy that edits what an agent sends is a
-- proxy whose logs stop explaining what happened.
--
-- The Lua filter has no JSON library, so both sides are matched with patterns
-- over the buffered body. That has two consequences worth stating plainly:
--
--   * A batch (`[{...},{...}]`) is classified by its first method only.
--   * The response body is only readable when Envoy buffered it. The buffer
--     filter buffers requests, not responses, so on most responses
--     `handle:body()` raises and the MCP result fields stay empty. The log then
--     still has the method, the tool, the session and the duration — what it
--     does not have is the result, and it does not pretend otherwise.
--
-- A wrong tool name in a log is worse than none, so anything unrecognised
-- stays empty.

-- This node's identity, filled in by start-egress-proxy.sh when the container
-- starts (the token is not in the image). Used for the tunnel check below.
local NODE_ID = "__NESTBOX_NODE_ID__"
local NODE_TOKEN = "__NESTBOX_NODE_TOKEN__"

local METHODS = {
  "initialize",
  "tools/",
  "resources/",
  "prompts/",
  "notifications/",
  "logging/",
  "completion/",
  "sampling/",
  "roots/",
  "elicitation/",
  "ping",
}

local function lower(value)
  return string.lower(value)
end

local function set(handle, name, value)
  if value == nil or value == "" then
    return
  end
  handle:streamInfo():dynamicMetadata():set("nestbox", name, value)
end

-- How much of a body is recorded. The control API keeps the same 64 KiB and
-- marks a longer body as truncated, so the proxy never ships more than that.
local BODY_LIMIT = 65536

-- The newest message of a chat request, cut out of a body that is too long to
-- keep whole.
--
-- Claude sends `messages` first and a very large `tools` list after it, so the
-- end of the body is not where the question is, and neither is the start. This
-- walks the `messages` array (strings and nesting respected), remembers where
-- its last element starts and ends, and returns a small JSON document with the
-- model, the number of messages and that last message. nil when the body has no
-- such array, so the caller falls back to head and tail.
local function last_message_digest(bytes)
  local _, array_open = string.find(bytes, '"messages"%s*:%s*%[')
  if not array_open then
    return nil
  end
  local pos, depth, count = array_open + 1, 1, 0
  local element_start, last_start, last_end
  while true do
    local i = string.find(bytes, '[%[%]{}"]', pos)
    if not i then
      return nil
    end
    local c = string.sub(bytes, i, i)
    if c == '"' then
      local j = i + 1
      while true do
        local q = string.find(bytes, '["\\]', j)
        if not q then
          return nil
        end
        if string.sub(bytes, q, q) == "\\" then
          j = q + 2
        else
          j = q + 1
          break
        end
      end
      pos = j
    else
      if c == "[" or c == "{" then
        if depth == 1 and c == "{" then
          count = count + 1
          element_start = i
        end
        depth = depth + 1
      else
        depth = depth - 1
        if depth == 1 and element_start then
          last_start, last_end = element_start, i
          element_start = nil
        end
        if depth == 0 then
          break
        end
      end
      pos = i + 1
    end
  end
  if not last_start then
    return nil
  end
  local model = string.match(bytes, '"model"%s*:%s*"([^"]+)"')
  -- Claude Code names its session inside `metadata.user_id`, which is itself a
  -- JSON string, so the quotes around the id are escaped. Kept so the console
  -- can put a conversation's calls together.
  local session = string.match(bytes, 'session_id\\"%s*:%s*\\"([%x%-]+)\\"') or string.match(bytes, '"session_id"%s*:%s*"([%x%-]+)"')
  return string.format(
    '{"model":%s,"nestbox_history":%d,"nestbox_conversation":%s,"messages":[%s]}',
    model and ('"' .. model .. '"') or "null",
    count,
    session and ('"' .. session .. '"') or "null",
    string.sub(bytes, last_start, last_end)
  )
end

-- Record a body for the access log as base64 under `name`.
local function record_body(handle, name, bytes)
  if bytes == nil or bytes == "" then
    return
  end
  if #bytes > BODY_LIMIT then
    local digest = name == "request_body" and last_message_digest(bytes) or nil
    if digest then
      bytes = digest
    elseif name == "request_body" then
      -- A long chat request carries the model and system prompt at the start
      -- and the newest message at the end; the middle is history. Keep both
      -- ends, so the console can still show what was asked just now.
      local head = 8192
      bytes = string.sub(bytes, 1, head) .. string.sub(bytes, -(BODY_LIMIT - head))
    else
      bytes = string.sub(bytes, 1, BODY_LIMIT)
    end
  end
  set(handle, name, handle:base64Escape(bytes))
end

-- ---------------------------------------------------------------------------
-- Model usage
--
-- A model call is recognised by its path. For such a call the proxy keeps three
-- facts and no content: how it was authenticated (a key or a signed-in account —
-- never the credential), how many `cache_control` markers the request carried, and
-- the counters the provider put in its answer. The counters travel in one
-- vocabulary whatever the provider says: `i` fresh input, `o` output, `cr` cache
-- reads, `cw` cache writes, `cw1` the one-hour share of the writes, `rs` reasoning.
-- ---------------------------------------------------------------------------

local MODEL_PATHS = {
  "/v1/messages", "/chat/completions", "/v1/responses", "/backend-api/codex/responses",
  "/v1/completions", ":generateContent", ":streamGenerateContent",
}

local function is_model_path(path)
  for _, needle in ipairs(MODEL_PATHS) do
    if string.find(path, needle, 1, true) then
      return true
    end
  end
  return string.find(path, "/responses", 1, true) ~= nil
end

-- Whose bill a call is on, from the shape of the credential. Only the class
-- leaves this function.
local function auth_kind(headers, host)
  if host == "chatgpt.com" or host == "chat.openai.com" then
    return "account"
  end
  local api_key = headers:get("x-api-key")
  if api_key and api_key ~= "" then
    if string.find(api_key, "sk-ant-oat", 1, true) == 1 then
      return "account"
    end
    return "key"
  end
  local authorization = headers:get("authorization")
  if authorization and authorization ~= "" then
    if string.find(authorization, "Bearer sk-ant-oat", 1, true) == 1 then
      return "account"
    end
    if string.find(authorization, "Bearer sk-", 1, true) == 1 then
      return "key"
    end
    return "account"
  end
  return "unknown"
end

local function count_plain(text, needle)
  local count, from = 0, 1
  while true do
    local at = string.find(text, needle, from, true)
    if not at then
      return count
    end
    count = count + 1
    from = at + #needle
  end
end

-- The last number written after `"key":` in either text (the tail wins: counters
-- are cumulative and the final event carries the totals).
local function last_number(head, tail, key)
  local pattern = '"' .. key .. '"%s*:%s*(%d+)'
  local found
  for _, text in ipairs({ head, tail }) do
    if text and text ~= "" then
      for n in string.gmatch(text, pattern) do
        found = n
      end
    end
  end
  return found and tonumber(found) or nil
end

-- The usage counters of an answer, as compact JSON, or nil when it has none.
local function extract_usage(head, tail)
  local cache_read = last_number(head, tail, "cache_read_input_tokens")
  local cache_write = last_number(head, tail, "cache_creation_input_tokens")
  local write_1h = last_number(head, tail, "ephemeral_1h_input_tokens")
  local input, output, reasoning = 0, 0, 0
  if cache_read or cache_write then
    -- Anthropic: `input_tokens` is only the fresh part.
    input = last_number(head, tail, "input_tokens") or 0
    output = last_number(head, tail, "output_tokens") or 0
    reasoning = last_number(head, tail, "thinking_tokens") or 0
  elseif last_number(head, tail, "prompt_tokens") then
    -- OpenAI chat: `prompt_tokens` includes what was cached.
    local prompt = last_number(head, tail, "prompt_tokens") or 0
    cache_read = last_number(head, tail, "cached_tokens") or 0
    input = math.max(prompt - cache_read, 0)
    output = last_number(head, tail, "completion_tokens") or 0
    reasoning = last_number(head, tail, "reasoning_tokens") or 0
  elseif last_number(head, tail, "promptTokenCount") then
    -- Gemini.
    local prompt = last_number(head, tail, "promptTokenCount") or 0
    cache_read = last_number(head, tail, "cachedContentTokenCount") or 0
    input = math.max(prompt - cache_read, 0)
    output = last_number(head, tail, "candidatesTokenCount") or 0
    reasoning = last_number(head, tail, "thoughtsTokenCount") or 0
  elseif last_number(head, tail, "input_tokens") then
    -- OpenAI responses / Codex (cached inside `input_tokens`), or Anthropic
    -- without caching (no cached field at all).
    local total = last_number(head, tail, "input_tokens") or 0
    cache_read = last_number(head, tail, "cached_tokens") or 0
    input = math.max(total - cache_read, 0)
    output = last_number(head, tail, "output_tokens") or 0
    reasoning = last_number(head, tail, "reasoning_tokens") or last_number(head, tail, "thinking_tokens") or 0
  else
    return nil
  end
  cache_read = cache_read or 0
  cache_write = cache_write or 0
  write_1h = math.min(write_1h or 0, cache_write)
  if input + output + cache_read + cache_write == 0 then
    return nil
  end
  return string.format(
    '{"i":%d,"o":%d,"cr":%d,"cw":%d,"cw1":%d,"rs":%d}',
    input, output, cache_read, cache_write, write_1h, reasoning
  )
end

-- Whether the request phase marked this call as a model call.
local function is_model_call(handle)
  local nestbox = handle:streamInfo():dynamicMetadata():get("nestbox")
  return nestbox ~= nil and nestbox.llm == "1"
end

local function json(headers)
  local content_type = headers:get("content-type")
  if not content_type then
    return false
  end
  return string.find(lower(content_type), "application/json", 1, true) ~= nil
end

-- Split an authority into host and port, honoring `[2001:db8::1]:443`.
local function split_authority(authority)
  if not authority then
    return nil, nil
  end
  authority = string.gsub(authority, "^%s*", "")
  authority = string.gsub(authority, "%s*$", "")
  if string.sub(authority, 1, 1) == "[" then
    local host, port = string.match(authority, "^%[([^%]]+)%]:?(%d*)$")
    if port == "" then
      port = nil
    end
    return host, port
  end
  local host, port = string.match(authority, "^([^:]+):?(%d*)$")
  if port == "" then
    port = nil
  end
  return host, port
end

local function is_mcp_method(method)
  for _, prefix in ipairs(METHODS) do
    if string.sub(method, 1, #prefix) == prefix then
      return true
    end
  end
  return false
end

-- Return the MCP method and, for `tools/call`, the tool name.
local function classify(body)
  -- `jsonrpc: "2.0"` first: without it this is just a body with a "method".
  if not string.find(body, '"jsonrpc"%s*:%s*"2%.0"') then
    return nil, nil
  end
  local method = string.match(body, '"method"%s*:%s*"([%w%._%-/]+)"')
  if not method or not is_mcp_method(method) then
    return nil, nil
  end
  if method ~= "tools/call" then
    return method, nil
  end
  -- `params.name` is the tool. Search after the `params` key so a `name`
  -- field in a preceding object cannot be mistaken for it.
  local after_method = string.match(body, '"method"%s*:%s*"tools/call"(.*)$')
  if not after_method then
    return method, nil
  end
  local after_params = string.match(after_method, '"params"%s*:%s*{(.*)$') or after_method
  local tool = string.match(after_params, '"name"%s*:%s*"([^"]+)"')
  return method, tool
end

-- Ask the control API whether a CONNECT may open, before a byte is tunnelled.
--
-- Envoy's ext_authz cannot carry a CONNECT to an HTTP authorization service (the
-- request has no path, and the service closes the connection), so the question is
-- asked here with a plain GET. The answer is the same one every other request
-- gets: the control API decides from the destination and the source address.
-- Anything but a clear "200" refuses the tunnel, including no answer at all: a
-- proxy that cannot ask does not guess.
local function check_tunnel(handle, authority, source_address)
  local ok, response_headers = pcall(function()
    local result = handle:httpCall(
      "control_api",
      {
        [":method"] = "GET",
        [":path"] = "/api/v1/internal/policy/check/",
        [":authority"] = authority,
        ["x-nestbox-node"] = NODE_ID,
        ["authorization"] = "Bearer " .. NODE_TOKEN,
        ["x-nestbox-source-ip"] = source_address or "",
        ["user-agent"] = handle:headers():get("user-agent") or "",
      },
      "",
      2000
    )
    return result
  end)
  local meta = handle:streamInfo():dynamicMetadata()
  if not ok or response_headers == nil then
    meta:set("envoy.filters.http.ext_authz", "x-nestbox-decision", "deny")
    meta:set("envoy.filters.http.ext_authz", "x-nestbox-reason", "decider_unreachable")
    handle:respond({ [":status"] = "503", ["x-nestbox-decision"] = "deny" }, "")
    return
  end
  for _, name in ipairs({ "x-nestbox-decision", "x-nestbox-reason", "x-nestbox-rule-id", "x-nestbox-sandbox", "x-nestbox-would-deny" }) do
    local value = response_headers[name]
    if value then
      meta:set("envoy.filters.http.ext_authz", name, value)
    end
  end
  -- `transparent` and `hybrid` mean this sandbox may have its TLS read. The
  -- answer is handed to the next filter as metadata, not written here: a header
  -- mutation made after this filter's `httpCall` does not clear the route cache,
  -- so the route would stay the one chosen for the request as it arrived. The
  -- filter in `route.lua` mutates the header while the headers are still
  -- continuable, which is what makes Envoy re-resolve the route.
  local mode = response_headers["x-nestbox-egress-mode"]
  handle:streamInfo():dynamicMetadata():set(
    "nestbox_route",
    "inspect",
    (mode == "transparent" or mode == "hybrid") and "1" or "0"
  )
  if response_headers[":status"] ~= "200" then
    handle:respond(
      { [":status"] = "403", ["x-nestbox-decision"] = response_headers["x-nestbox-decision"] or "deny" },
      ""
    )
  end
end

function envoy_on_request(handle)
  -- Who is asking is the connection's address, not anything the client wrote.
  -- Any `x-nestbox-*` header the sandbox sent is removed first, then the source
  -- address is set from the socket, so a sandbox cannot claim to be another.
  local request_headers = handle:headers()
  request_headers:remove("x-nestbox-sandbox")
  request_headers:remove("x-nestbox-sandbox-id")
  request_headers:remove("x-nestbox-node")
  request_headers:remove("x-nestbox-node-token")
  request_headers:remove("x-nestbox-source-ip")
  request_headers:remove("x-nestbox-inspect")
  -- The restored address when a PROXY header gave one (the decrypted hop from
  -- the certificate broker), the socket's own address otherwise.
  local info = handle:streamInfo()
  local peer = info:downstreamRemoteAddress() or info:downstreamDirectRemoteAddress()
  if peer then
    local address = string.match(peer, "^(.-):%d+$") or peer
    request_headers:add("x-nestbox-source-ip", address)
  end
  local headers = handle:headers()

  local host, port = split_authority(headers:get(":authority"))
  set(handle, "target_host", host)
  set(handle, "target_port", port)

  if headers:get(":method") == "CONNECT" then
    -- A tunnel has no HTTP request inside that this listener can describe, and
    -- no visible ClientHello. What we do know is the name the client asked
    -- for; the log says `connect: true` so nobody reads this as a sniffed SNI.
    set(handle, "connect", "true")
    check_tunnel(handle, headers:get(":authority"), request_headers:get("x-nestbox-source-ip"))
    set(handle, "sni", host)
    -- httpCall yields; header objects fetched before it are invalid after it.
    headers = handle:headers()
  end

  set(handle, "mcp_session_id", headers:get("mcp-session-id"))

  if headers:get(":method") ~= "CONNECT" then
    -- Ask the server for an uncompressed answer so the recorded response is
    -- readable. A compressed body in the log would be noise.
    headers:remove("accept-encoding")
    local model_call = is_model_path(headers:get(":path") or "")
    local seen, buffered = pcall(function() return handle:body() end)
    if seen and buffered ~= nil and buffered:length() > 0 then
      local text = buffered:getBytes(0, buffered:length())
      if model_call then
        local markers = count_plain(text, '"cache_control"')
        if markers > 0 then
          set(handle, "cache_markers", tostring(markers))
        end
      end
      record_body(handle, "request_body", text)
    end
    -- Reading the body yields; the header object from before it is stale.
    headers = handle:headers()
    if model_call then
      set(handle, "llm", "1")
      set(handle, "llm_auth", auth_kind(headers, host))
    end
  end

  if not json(headers) then
    return
  end

  -- Envoy raises when the body was not buffered (over the limit, or a stream).
  -- That is not an error worth failing the request for: it is a body this
  -- filter cannot see, and the log will show empty MCP fields.
  local ok, body = pcall(function() return handle:body() end)
  if not ok or body == nil then
    return
  end
  local method, tool = classify(body:getBytes(0, body:length()))
  if not method then
    return
  end
  set(handle, "app_protocol", "mcp")
  set(handle, "mcp_method", method)
  set(handle, "mcp_tool", tool)
end

function envoy_on_response(handle)
  local headers = handle:headers()

  -- Servers assign the session on `initialize` and return it as a header. This
  -- is the only MCP fact that shows up on the response side without a body.
  local session = headers:get("mcp-session-id")
  if session then
    set(handle, "mcp_session_id", session)
  end

  local content_type = headers:get("content-type")
  if not content_type or string.find(lower(content_type), "application/json", 1, true) == nil then
    -- Streamed answers (server-sent events, chunked chat replies) are read
    -- chunk by chunk and passed on untouched, so recording does not delay the
    -- first token. Only the first BODY_LIMIT bytes are kept.
    local kept, size = {}, 0
    local model_call = is_model_call(handle)
    local tail = ""
    for chunk in handle:bodyChunks() do
      local n = chunk:length()
      if size < BODY_LIMIT and n > 0 then
        kept[#kept + 1] = chunk:getBytes(0, math.min(n, BODY_LIMIT - size))
        size = size + math.min(n, BODY_LIMIT - size)
      end
      if model_call and n > 0 then
        -- The totals come in the last event, which a long answer pushes past
        -- the kept head; the last 16 KiB are enough to hold it.
        tail = string.sub(tail .. chunk:getBytes(0, n), -16384)
      end
    end
    local head = table.concat(kept)
    record_body(handle, "response_body", head)
    if model_call then
      set(handle, "usage", extract_usage(head, tail))
    end
    return
  end

  local ok, body = pcall(function() return handle:body() end)
  if not ok or body == nil then
    return
  end
  local text = body:getBytes(0, body:length())
  if text == nil or text == "" then
    return
  end
  record_body(handle, "response_body", text)
  if is_model_call(handle) then
    set(handle, "usage", extract_usage(text, ""))
  end
  if string.find(text, '"isError"%s*:%s*true') then
    set(handle, "mcp_is_error", "true")
    set(handle, "mcp_result_kind", "tool_error")
  elseif string.find(text, '"error"%s*:') then
    set(handle, "mcp_is_error", "true")
    set(handle, "mcp_result_kind", "error")
  elseif string.find(text, '"result"%s*:') then
    set(handle, "mcp_is_error", "false")
    set(handle, "mcp_result_kind", "result")
  end
end
