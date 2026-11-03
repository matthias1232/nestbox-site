import test from 'node:test';
import assert from 'node:assert/strict';
import {
  addLine, allowAllChoice, channelAction, draftsSentence, filterLines, labelFor, laneAlerts,
  logLine, mayControl, nextAllowAllAction, recordingAction, rulesSentence, stripState,
  summarizeTraffic, windowFor,
} from './controlStrip.js';

const later = new Date(Date.now() + 600000).toISOString();

test('a quiet summary has everything off and the forward proxy', () => {
  const state = stripState({});
  assert.equal(state.allowAll.on, false);
  assert.equal(state.mode, 'forward_proxy');
  assert.equal(state.recording, 'full');
  assert.equal(state.channels.dns.on, false);
});

test('a whole-sandbox window means allow-all with its end', () => {
  const state = stripState({ windows: [{ channel: 'all', mode: 'bypass', expires_at: later }] });
  assert.equal(state.allowAll.on, true);
  assert.equal(state.allowAll.until, later);
  assert.equal(state.channels.mcp.on, true);
});

test('a dns window opens only the dns switch', () => {
  const state = stripState({ windows: [{ channel: 'dns', expires_at: later }] });
  assert.equal(state.allowAll.on, false);
  assert.equal(state.channels.dns.on, true);
  assert.equal(state.channels.internet.on, false);
});

test('proxy off counts as allow-all', () => {
  assert.equal(stripState({ egress: { mode: 'off_allow_all' } }).allowAll.on, true);
});

test('recording comes from the mute columns when the summary has no field', () => {
  assert.equal(stripState({ telemetry_mute_until: later }).recording, 'off');
  assert.equal(stripState({ telemetry_mute_bodies_until: later }).recording, 'flows');
  assert.equal(stripState({ recording: 'flows' }).recording, 'flows');
});

test('the allow-all click ends a running one and starts 15 minutes otherwise', () => {
  assert.deepEqual(nextAllowAllAction({ allowAll: { on: true } }), { what: 'allow_all', value: false, minutes: null });
  assert.deepEqual(nextAllowAllAction({ allowAll: { on: false } }), { what: 'allow_all', value: true, minutes: 15 });
  assert.deepEqual(nextAllowAllAction({}, '60'), { what: 'allow_all', value: true, minutes: 60 });
  assert.deepEqual(nextAllowAllAction({}, 'forever'), { what: 'allow_all', value: true, minutes: null });
});

test('the pressed allow-all choice', () => {
  assert.equal(allowAllChoice({ allowAll: { on: false } }), 'off');
  assert.equal(allowAllChoice({ allowAll: { on: true, until: null } }), 'forever');
  assert.equal(allowAllChoice({ allowAll: { on: true, until: later } }, '60'), '60');
  assert.equal(allowAllChoice({ allowAll: { on: true, until: later } }), null);
});

test('labels carry the recording sentence', () => {
  assert.equal(labelFor('mode', 'hybrid'), 'Hybrid');
  assert.match(labelFor('recording', 'off'), /store nothing/);
});

test('a member controls only their own box', () => {
  assert.equal(mayControl('member', 'a', 'b'), false);
  assert.equal(mayControl('member', 'a', 'a'), true);
  assert.equal(mayControl('operator', 'a', 'b'), true);
});

test('quick call bodies', () => {
  assert.deepEqual(recordingAction('flows'), { what: 'recording', value: 'flows', minutes: 1440 });
  assert.deepEqual(channelAction('dns', true, 60), { what: 'dns', value: true, minutes: 60 });
  assert.deepEqual(channelAction('dns', false, 60), { what: 'dns', value: false, minutes: null });
});

test('log lines of another sandbox are dropped, refused ones are marked', () => {
  const event = { id: 'e1', sandbox_id: 's1', decision: 'deny', destination: 'x.io', port: 443, ts: '2026-01-01T00:00:00Z' };
  assert.equal(logLine('network', event, 's2'), null);
  const line = logLine('network', event, 's1');
  assert.equal(line.refused, true);
  assert.equal(line.text, 'x.io:443');
  assert.equal(logLine('dns', { id: 'd', sandbox_id: 's1', hostname: 'a.b' }, 's1').text, 'a.b');
});

test('lines are newest first, bounded and not doubled', () => {
  let lines = [];
  lines = addLine(lines, { key: 'a' });
  lines = addLine(lines, { key: 'b' });
  lines = addLine(lines, { key: 'a', v: 2 });
  assert.deepEqual(lines.map((l) => l.key), ['a', 'b']);
  for (let i = 0; i < 400; i += 1) lines = addLine(lines, { key: `k${i}` });
  assert.equal(lines.length, 300);
});

test('filters', () => {
  const lines = [
    { kind: 'network', refused: true }, { kind: 'dns', refused: false }, { kind: 'inference', refused: false },
  ];
  assert.equal(filterLines(lines, 'all').length, 3);
  assert.equal(filterLines(lines, 'refused').length, 1);
  assert.equal(filterLines(lines, 'dns')[0].kind, 'dns');
  assert.equal(filterLines(lines, 'inference')[0].kind, 'inference');
});

test('the red lane keeps one alert per id and hides ignored ones', () => {
  const alerts = [
    { id: 'a', sandbox_id: 's', last_at: '2026-01-01T00:00:00Z', count: 1 },
    { id: 'a', sandbox_id: 's', last_at: '2026-01-01T00:01:00Z', count: 2 },
    { id: 'b', sandbox_id: 's', last_at: '2026-01-01T00:00:30Z' },
    { id: 'c', sandbox_id: 'other', last_at: '2026-01-01T00:00:30Z' },
  ];
  const lane = laneAlerts(alerts, 's', (id) => id === 'b');
  assert.deepEqual(lane.map((a) => a.id), ['a']);
  assert.equal(lane[0].count, 2);
});

test('traffic is summarized by distinct destination', () => {
  const summary = summarizeTraffic([
    { host: 'a.io', port: 443, decision: 'allow' },
    { host: 'a.io', port: 443, decision: 'deny' },
    { host: 'b.io', port: 80, would_deny: true },
    { host: 'c.io', port: 80, decision: 'allow' },
  ]);
  assert.deepEqual(summary, { seen: 3, refused: 2 });
  assert.deepEqual(summarizeTraffic(null), { seen: 0, refused: 0 });
});

test('windows and sentences', () => {
  const now = Date.parse('2026-01-01T12:00:00Z');
  assert.equal(windowFor('2026-01-01T11:50:00Z', now), '15m');
  assert.equal(windowFor('2026-01-01T11:20:00Z', now), '1h');
  assert.equal(windowFor('2025-12-31T00:00:00Z', now), '24h');
  assert.match(rulesSentence({ seen: 37, refused: 9 }, '2026-01-01T12:00:00Z', true), /37 destinations were seen, 9 would have been refused/);
  assert.match(rulesSentence({ seen: 1, refused: 0 }, '2026-01-01T12:00:00Z', false), /last 24 hours, 1 destination was seen/);
  assert.equal(draftsSentence({ created: 3 }), '3 drafts created, review them');
});

import { notYetGranted } from './controlStrip.js';

test('only models the sandbox lacks are offered, once each', () => {
  const offered = notYetGranted(
    [{ model_name: 'a' }, { model_name: 'b' }, { model_name: 'b' }, { model_name: 'c' }],
    [{ model_name: 'a' }],
  );
  assert.deepEqual(offered.map((g) => g.model_name), ['b', 'c']);
});
