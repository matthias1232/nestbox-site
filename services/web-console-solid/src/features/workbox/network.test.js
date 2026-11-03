import test from 'node:test';
import assert from 'node:assert/strict';

import {
  DURATIONS,
  MODES,
  allowAllSentence,
  allowAllUntil,
  channelsFor,
  draftSince,
  openWindows,
} from './network.js';

test('nothing is allowed when the mode is hybrid and no window is open', () => {
  const sentence = allowAllSentence({ allow_all: false, egress: { mode: 'hybrid' }, windows: [] });
  assert.equal(sentence, null);
});

test('a proxy that is off says so and that it only records', () => {
  const sentence = allowAllSentence({ allow_all: true, egress: { mode: 'off_allow_all' }, windows: [] });
  assert.match(sentence, /Proxy is off/);
});

test('a window on OpenShell mentions OpenShell', () => {
  const sentence = allowAllSentence({
    allow_all: true,
    egress: { mode: 'hybrid' },
    windows: [{ id: 'w1', mode: 'audit', channel: 'openshell', expires_at: null, scope: 'sandbox' }],
  });
  assert.match(sentence, /OpenShell/);
});

test('a window for everything names all four firewalls', () => {
  const sentence = allowAllSentence({
    allow_all: true,
    egress: { mode: 'forward_proxy' },
    windows: [{ id: 'w1', mode: 'bypass', channel: 'all', expires_at: null, scope: 'sandbox' }],
  });
  assert.match(sentence, /network, DNS, MCP and OpenShell/);
});

test('an expired window no longer allows anything', () => {
  const windows = [
    { id: 'w1', mode: 'audit', channel: 'all', expires_at: new Date(Date.now() - 60_000).toISOString() },
  ];
  assert.equal(openWindows({ windows }).length, 0);
  assert.equal(allowAllSentence({ allow_all: false, egress: { mode: 'hybrid' }, windows }), null);
});

test('four ticked firewalls become one window for everything', () => {
  assert.deepEqual(channelsFor(['openshell', 'proxy', 'dns', 'mcp']), ['all']);
  assert.deepEqual(channelsFor(['dns']), ['dns']);
  assert.deepEqual(channelsFor(['mcp', 'proxy']), ['proxy', 'mcp']);
  assert.deepEqual(channelsFor([]), []);
});

test('a window without an end has no countdown target', () => {
  const windows = [{ id: 'w1', mode: 'audit', channel: 'all', expires_at: null }];
  assert.equal(allowAllUntil({ allow_all: true, egress: { mode: 'hybrid' }, windows }), null);
  const timed = [{ id: 'w2', mode: 'audit', channel: 'all', expires_at: new Date(Date.now() + 3_600_000).toISOString() }];
  assert.equal(allowAllUntil({ allow_all: true, egress: { mode: 'hybrid' }, windows: timed }), timed[0].expires_at);
});

test('without a window start the drafts look back 24 hours', () => {
  const now = new Date('2026-11-01T12:00:00Z');
  const since = draftSince({ windows: [{ id: 'w1', mode: 'audit', channel: 'all', expires_at: null }] }, now);
  assert.equal(since, '2026-10-31T12:00:00.000Z');
});

test('the mode list is the four user modes, with a badge tone on the open one', () => {
  assert.deepEqual(MODES.map((item) => item.value), ['transparent', 'hybrid', 'forward_proxy', 'off_allow_all']);
  assert.equal(MODES.find((item) => item.value === 'off_allow_all').tone, 'bad');
});

test('a duration exists for unlimited and nothing else is null', () => {
  assert.deepEqual(DURATIONS.map((item) => item.minutes), [15, 60, 240, 480, null]);
});
