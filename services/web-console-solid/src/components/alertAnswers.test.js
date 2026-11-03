import test from 'node:test';
import assert from 'node:assert/strict';
import { answers, allowBody, windowBody, draftEvent, ageOf } from './alertAnswers.js';

const alert = {
  id: 'a1', sandbox_id: 's1', sandbox_name: 'research-01', host: 'api.example.net', port: 443,
  protocol: 'https', channel: 'proxy', count: 7, decision: 'blocked', reason: 'no_rule',
};

test('an alert offers the six answers in order', () => {
  assert.deepEqual(answers(alert).map((a) => a.id), ['now', '15', '60', 'draft', 'ignore', 'open']);
});

test('the allow body carries host, port and protocol', () => {
  assert.deepEqual(allowBody(alert), { host: 'api.example.net', port: 443, protocol: 'https' });
});

test('a window body is a bounded bypass on the alert channel', () => {
  const body = windowBody(alert, '15');
  assert.equal(body.minutes, 15);
  assert.equal(body.unlimited, false);
  assert.equal(body.mode, 'bypass');
  assert.equal(body.channel, 'proxy');
  assert.equal(body.reason, 'alert api.example.net');
});

test('an unknown channel opens the window on all channels', () => {
  assert.equal(windowBody({ ...alert, channel: 'gateway' }, 60).channel, 'all');
});

test('the draft event keeps the destination', () => {
  assert.equal(draftEvent(alert).host, 'api.example.net');
});

test('age reads in seconds, minutes and hours', () => {
  const now = Date.parse('2026-11-03T12:00:00Z');
  assert.equal(ageOf('2026-11-03T11:59:30Z', now), '30 s ago');
  assert.equal(ageOf('2026-11-03T11:57:00Z', now), '3 min ago');
  assert.equal(ageOf('2026-11-03T10:00:00Z', now), '2 h ago');
  assert.equal(ageOf('nope', now), '');
});
