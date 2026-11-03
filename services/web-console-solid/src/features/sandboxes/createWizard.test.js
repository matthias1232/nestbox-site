import test from 'node:test';
import assert from 'node:assert/strict';
import { blankState, stepsFor, validate, bodyFor, followUps, nodeHint, leastLoaded, createCall, isDigest } from './createWizard.js';

const GIB = 1024 ** 3;
const digest = `sha256:${'a'.repeat(64)}`;
const node = (name, freeGib, extra = {}) => ({
  name, status: 'online', hardware_known: true, budget: { ram_free_bytes: freeGib * GIB, cpu_cores: 8, gpu_count: 0 }, ...extra,
});

test('a recipe skips the Where step', () => {
  assert.deepEqual(stepsFor('recipe').map((s) => s.id), ['what', 'access', 'review']);
  assert.deepEqual(stepsFor('pack').map((s) => s.id), ['what', 'where', 'access', 'review']);
});

test('the What step needs a name and a pack', () => {
  const state = { ...blankState() };
  assert.equal(validate('what', state).length, 2);
  assert.deepEqual(validate('what', { ...state, name: 'a', pack_key: 'hermes' }), []);
});

test('an image needs a digest, a tag is refused', () => {
  const state = { ...blankState(), source: 'image', name: 'a', pack_key: 'hermes', image: 'reg/x:latest' };
  assert.equal(validate('what', state).length, 1);
  assert.deepEqual(validate('what', { ...state, image_digest: digest }), []);
  assert.equal(isDigest('latest'), false);
});

test('bodyFor sends MiB, the node and the egress mode', () => {
  const body = bodyFor({ ...blankState(), name: ' box ', pack_key: 'claude-code', memory_gib: '0.5', node: 'gpu-1', login: 'own_account' });
  assert.equal(body.name, 'box');
  assert.equal(body.memory_limit, '512Mi');
  assert.equal(body.node, 'gpu-1');
  assert.equal(body.login, 'own_account');
  assert.equal(body.egress_mode, 'forward_proxy');
  assert.equal('gpu_count' in body, false);
});

test('own_account is dropped for a pack without its own login', () => {
  const body = bodyFor({ ...blankState(), name: 'a', pack_key: 'hermes', login: 'own_account' });
  assert.equal('login' in body, false);
});

test('a recipe is created through instantiate', () => {
  const made = createCall({ ...blankState(), source: 'recipe', recipe_id: 'r1', name: 'x' });
  assert.deepEqual(made.call, { api: 'recipes.instantiate', args: ['r1', 'x'] });
});

test('follow-ups run grants, then policy, then mounts', () => {
  const state = {
    ...blankState(), models: ['m1', 'm2'], policy_id: 'p1',
    mounts: [{ root_id: 'r1', mount_path: '/work', read_only: false }],
  };
  const calls = followUps(state, 's1', { m1: { provider_profile_id: 'pp' } });
  assert.deepEqual(calls.map((c) => c.call.api), ['modelGrants.create', 'modelGrants.create', 'policies.assignPolicy', 'mounts.put']);
  assert.equal(calls[0].call.args[0].sandbox_id, 's1');
  assert.equal(calls[3].call.args[1].apply, 'now');
  assert.deepEqual(followUps(blankState(), 's1'), []);
});

test('duplicate and relative mount paths are refused', () => {
  const mounts = [{ mount_path: '/a' }, { mount_path: '/a' }, { mount_path: 'b' }];
  assert.equal(validate('access', { ...blankState(), mounts }).length, 2);
});

test('nodeHint says fits, tight, too big and no GPU', () => {
  const state = { ...blankState(), memory_gib: '2' };
  assert.equal(nodeHint(node('a', 16), state).text, 'fits');
  assert.equal(nodeHint(node('a', 2.2), state).text, 'tight');
  assert.equal(nodeHint(node('a', 1), state).text, 'too big');
  assert.equal(nodeHint(node('a', 16), { ...state, gpu_count: '1' }).text, 'no GPU');
  assert.equal(nodeHint(node('a', 16, { container_gpu: true }), { ...state, gpu_count: '1' }).text, 'fits');
  assert.equal(nodeHint(node('a', 16, { hardware_known: false }), state).text, 'not measured');
});

test('least loaded picks the fitting node with most free memory', () => {
  const state = { ...blankState(), memory_gib: '2' };
  const picked = leastLoaded([node('a', 8), node('b', 32), node('c', 1)], state);
  assert.equal(picked.name, 'b');
  assert.equal(leastLoaded([node('c', 1)], state), null);
});
