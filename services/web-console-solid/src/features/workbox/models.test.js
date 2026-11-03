import test from 'node:test';
import assert from 'node:assert/strict';

import {
  canManageModels,
  readinessBadge,
  readinessFromEvents,
  sourceLabel,
} from './models.js';

test('the last readiness event of a model wins', () => {
  const readiness = readinessFromEvents([
    { type: 'models_changed', detail: { model: 'fast', state: 'warming', ms: null } },
    { type: 'models_changed', detail: { model: 'fast', state: 'ready', ms: 812 } },
  ]);
  assert.deepEqual(readiness.fast, { state: 'ready', ms: 812 });
});

test('readiness is kept per model, not per sandbox', () => {
  const readiness = readinessFromEvents([
    { type: 'models_changed', detail: { model: 'a', state: 'ready', ms: 10 } },
    { type: 'models_changed', detail: { model: 'b', state: 'failed', ms: null } },
  ]);
  assert.deepEqual(Object.keys(readiness).sort(), ['a', 'b']);
  assert.equal(readiness.b.state, 'failed');
});

test('events without a model and control events are ignored', () => {
  const readiness = readinessFromEvents([
    { type: 'sandbox_control_changed', what: 'agent_auth', detail: { model: 'x', state: 'ready' } },
    { type: 'models_changed', detail: { state: 'ready', ms: 5 } },
    { type: 'models_changed', detail: null },
    null,
  ]);
  assert.deepEqual(readiness, {});
});

test('a missing millisecond figure is not turned into zero', () => {
  const readiness = readinessFromEvents([
    { type: 'models_changed', detail: { model: 'a', state: 'ready' } },
  ]);
  assert.equal(readiness.a.ms, null);
});

test('every grant scope has a readable label', () => {
  assert.equal(sourceLabel('sandbox'), 'Sandbox');
  assert.equal(sourceLabel('user'), 'User');
  assert.equal(sourceLabel('environment'), 'Environment');
});

test('an unknown scope is capitalised instead of dropped', () => {
  assert.equal(sourceLabel('team'), 'Team');
  assert.equal(sourceLabel(''), 'Unknown');
  assert.equal(sourceLabel(null), 'Unknown');
});

test('warming, ready and broken readiness each get their own badge', () => {
  assert.deepEqual(readinessBadge({ state: 'warming', ms: null }), { tone: 'warn', text: 'warming up' });
  assert.deepEqual(readinessBadge({ state: 'ready', ms: 812 }), { tone: 'ok', text: '812 ms' });
  assert.deepEqual(readinessBadge({ state: 'ready', ms: null }), { tone: 'ok', text: 'ready' });
  assert.deepEqual(readinessBadge({ state: 'failed', ms: null }), { tone: 'bad', text: 'failed' });
  assert.deepEqual(readinessBadge({ state: 'missing', ms: null }), { tone: 'bad', text: 'missing' });
});

test('no readiness event means no badge', () => {
  assert.equal(readinessBadge(null), null);
  assert.equal(readinessBadge({}), null);
});

test('operators and admins manage models anywhere', () => {
  assert.equal(canManageModels('operator', null, 'u1'), true);
  assert.equal(canManageModels('admin', null, 'u1'), true);
});

test('a member manages models only on a sandbox they own', () => {
  assert.equal(canManageModels('member', 'u1', 'u1'), true);
  assert.equal(canManageModels('member', 'u2', 'u1'), false);
  assert.equal(canManageModels('member', null, 'u1'), false);
});

test('a viewer never manages models, not even as the owner', () => {
  assert.equal(canManageModels('viewer', 'u1', 'u1'), false);
});
