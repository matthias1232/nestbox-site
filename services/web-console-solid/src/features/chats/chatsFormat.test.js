import test from 'node:test';
import assert from 'node:assert/strict';
import {
  formatTokens,
  groupCounters,
  modelLabel,
  normaliseToolCalls,
  providerLabel,
  threadHeader,
  tokenLine,
  toolText,
} from './chatsFormat.js';

test('tokens are grouped with dots, as the console writes numbers', () => {
  assert.equal(formatTokens(0), '0');
  assert.equal(formatTokens(940), '940');
  assert.equal(formatTokens(1204), '1.204');
  assert.equal(formatTokens(1234567), '1.234.567');
  assert.equal(formatTokens(null), '0');
});

test('the token line is input, output, then cache', () => {
  assert.equal(tokenLine({ input_tokens: 1204, output_tokens: 318 }), '1.204 ein → 318 aus');
  assert.equal(
    tokenLine({ input_tokens: 1204, output_tokens: 318, cache_read: 96, cache_write: 40 }),
    '1.204 ein → 318 aus · 96 im Cache · 40 geschrieben',
  );
  assert.equal(tokenLine(null), '');
});

test('the model falls back through requested, router and served', () => {
  assert.equal(modelLabel({ served_model: 'served', requested_model: 'asked' }), 'served');
  assert.equal(modelLabel({ requested_model: 'asked', router_model: 'routed' }), 'asked');
  assert.equal(modelLabel({ router_model: 'routed' }), 'routed');
  assert.equal(modelLabel({}), '—');
  assert.equal(providerLabel({ provider: 'anthropic' }), 'anthropic');
  assert.equal(providerLabel({ router_provider: 'openrouter' }), 'openrouter');
});

test('tool calls become one shape whatever the provider sent', () => {
  assert.deepEqual(
    normaliseToolCalls([
      { name: 'read_file', input: { path: 'a' } },
      { function: { name: 'bash', arguments: '{"cmd":"ls"}' } },
      'plain_tool',
    ]),
    [
      { name: 'read_file', input: { path: 'a' } },
      { name: 'bash', input: '{"cmd":"ls"}' },
      { name: 'plain_tool', input: null },
    ],
  );
  assert.deepEqual(normaliseToolCalls(null), []);
  assert.deepEqual(normaliseToolCalls({ calls: [{ name: 'x' }] }), [{ name: 'x', input: null }]);
});

test('a tool call reads as one monospace line', () => {
  assert.equal(toolText({ name: 'read_file', input: { path: 'a' } }), 'read_file {"path":"a"}');
  assert.equal(toolText({ name: 'bash', input: null }), 'bash');
});

test('the thread head says who and where, once per value', () => {
  const turns = [
    { ts: '2026-10-01T10:00:00Z', sandbox_name: 'dev', owner_name: 'Ada', agent: 'claude', account_kind: 'api_key', provider: 'anthropic', served_model: 'claude-sonnet' },
    { ts: '2026-10-01T10:05:00Z', sandbox_name: 'dev', owner_name: 'Ada', agent: 'codex', account_kind: 'subscription', provider: 'openai', requested_model: 'gpt-5' },
  ];
  const head = threadHeader(turns);
  assert.equal(head.count, 2);
  assert.deepEqual(head.sandboxes, ['dev']);
  assert.deepEqual(head.users, ['Ada']);
  assert.deepEqual(head.agents, ['claude', 'codex']);
  assert.deepEqual(head.accounts, ['api_key', 'subscription']);
  assert.deepEqual(head.models, ['claude-sonnet', 'gpt-5']);
  assert.equal(head.first, '2026-10-01T10:00:00.000Z');
  assert.equal(head.last, '2026-10-01T10:05:00.000Z');
});

test('an empty thread has no head to show', () => {
  const head = threadHeader([]);
  assert.equal(head.count, 0);
  assert.deepEqual(head.models, []);
  assert.equal(head.first, null);
});

test('every group carries the three finding counters, zero included', () => {
  assert.deepEqual(groupCounters({ blocked: 2, replaced: 0, pseudonymized: 5 }), [
    { state: 'blocked', label: 'blockiert', value: 2 },
    { state: 'replaced', label: 'ersetzt', value: 0 },
    { state: 'pseudonymized', label: 'pseudonymisiert', value: 5 },
  ]);
});
