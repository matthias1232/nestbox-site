import test from 'node:test';
import assert from 'node:assert/strict';
import { agentLabel, busyMessage, loginHint, loginLine, modeLabel, sessionSummary } from './agents.js';

const claude = {
  agent: 'claude',
  mode: 'gateway',
  pending_restart: false,
  busy: false,
  sessions: [],
  signed_in: null,
  account: null,
};

const busySession = (name, idle) => ({ tmux_session: name, pane_id: `%${name}`, idle_seconds: idle, busy: true });
const idleSession = (name, idle) => ({ tmux_session: name, pane_id: `%${name}`, idle_seconds: idle, busy: false });

test('nothing running means no warning at all', () => {
  assert.equal(busyMessage(claude), null);
  assert.equal(busyMessage(null), null);
});

test('two working sessions are counted and named', () => {
  const message = busyMessage({ ...claude, sessions: [busySession('t-ab12', 3), busySession('t-cd34', 8)] });
  assert.ok(message.includes('2 sessions'), message);
  assert.ok(message.includes('t-ab12'), message);
  assert.ok(message.includes('t-cd34'), message);
  assert.ok(message.includes('stops their current answer'), message);
});

test('one working session is singular', () => {
  const message = busyMessage({ ...claude, sessions: [busySession('t-ab12', 3)] });
  assert.ok(message.includes('1 session is'), message);
  assert.ok(message.includes('its current answer'), message);
});

test('a busy agent without a pane is still named', () => {
  assert.ok(busyMessage({ ...claude, busy: true }).includes('working right now'));
});

test('idle sessions say that a restart does not interrupt an answer', () => {
  const message = busyMessage({ ...claude, sessions: [idleSession('t-ab12', 45)] });
  assert.ok(message.includes('open and idle'), message);
  assert.ok(message.includes('does not interrupt'), message);
});

test('the login line names the account when there is one', () => {
  assert.equal(loginLine({ signed_in: true, account: 'a@b' }), 'Signed in · a@b');
  assert.equal(loginLine({ signed_in: true, account: null }), 'Signed in');
  assert.equal(loginLine({ signed_in: false }), 'Not signed in');
  assert.equal(loginLine({ signed_in: null }), 'Login unknown');
  assert.equal(loginLine(null), 'Login unknown');
});

test('the subscription hint appears only for a missing login', () => {
  assert.match(loginHint({ ...claude, mode: 'subscription', signed_in: false }), /claude login/);
  assert.match(loginHint({ ...claude, mode: 'subscription', signed_in: false }), /\/login/);
  assert.equal(loginHint({ ...claude, mode: 'gateway', signed_in: false }), null);
  assert.equal(loginHint({ ...claude, mode: 'subscription', signed_in: true, account: 'a@b' }), null);
});

test('an agent id has a label and a mode has a word', () => {
  assert.equal(agentLabel('claude'), 'Claude Code');
  assert.equal(agentLabel('opencode'), 'OpenCode');
  assert.equal(agentLabel('hermes'), 'hermes');
  assert.equal(modeLabel('gateway'), 'Gateway');
  assert.equal(modeLabel('subscription'), 'Subscription');
});

test('the session line names each session and its state', () => {
  assert.equal(sessionSummary(claude), null);
  assert.equal(
    sessionSummary({ ...claude, sessions: [busySession('t-ab12', 0), idleSession('t-cd34', 45)] }),
    '2 sessions: t-ab12 (working), t-cd34 (idle for 45 s)',
  );
  assert.equal(
    sessionSummary({ ...claude, sessions: [idleSession('t-cd34', 300)] }),
    '1 session: t-cd34 (idle for 5 min)',
  );
});
