import test from 'node:test';
import assert from 'node:assert/strict';
import { isModelCall, readRequest, readResponse, sessionOf, titleOf } from './chats.js';

test('recognises model calls by path', () => {
    assert.equal(isModelCall({ method: 'POST', path: '/v1/messages' }), true);
    assert.equal(isModelCall({ method: 'GET', path: '/v1/messages' }), false);
    assert.equal(isModelCall({ method: 'POST', path: '/v1/embeddings' }), false);
  });
test('reads the newest user message', () => {
    const body = JSON.stringify({ model: 'm', messages: [{ role: 'user', content: 'a' }, { role: 'assistant', content: 'b' }, { role: 'user', content: [{ type: 'text', text: 'c' }] }] });
    assert.deepEqual(pick(readRequest(body), { model: 'm', prompt: 'c', history: 3 }), { model: 'm', prompt: 'c', history: 3 });
  });
test('salvages a request cut at the limit', () => {
    const cut = '{"model":"m","messages":[{"role":"user","content":"hello \\"x\\""},{"role":"user","content":"newest"},{"role":"us';
    assert.deepEqual(pick(readRequest(cut), { model: 'm', prompt: 'newest', partial: true }), { model: 'm', prompt: 'newest', partial: true });
  });
test('stitches a streamed answer', () => {
    const sse = 'data: {"type":"content_block_delta","delta":{"text":"He"}}\n\ndata: {"type":"content_block_delta","delta":{"text":"llo"}}\n';
    assert.equal(readResponse(sse).text, 'Hello');
  });
test('reads an error answer', () => {
    assert.equal(readResponse('{"type":"error","error":{"message":"bad key"}}').error, 'bad key');
  });

function pick(actual, expected) {
  return Object.fromEntries(Object.keys(expected).map((key) => [key, actual[key]]));
}

test('finds the session of a Claude Code call and a title answer', () => {
  const raw = '{"metadata":{"user_id":"{\\"device_id\\":\\"d\\",\\"session_id\\":\\"27a41161-b693-419a-850f-a5f2565bd848\\"}"}}';
  assert.equal(sessionOf(raw), '27a41161-b693-419a-850f-a5f2565bd848');
  assert.equal(sessionOf('{"nestbox_conversation":"abcd1234-aa","messages":[]}'), 'abcd1234-aa');
  assert.equal(sessionOf('{"messages":[]}'), null);
  assert.equal(titleOf('{"title": "Fix the \\"login\\" bug"}'), 'Fix the "login" bug');
  assert.equal(titleOf('hello'), null);
});
