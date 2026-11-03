import test from 'node:test';
import assert from 'node:assert/strict';
import { blankState, steps, validate, bodyFor, reviewLines, progressStates, nameProblem, deployableModels, firstProblem } from './nodeWizard.js';

test('script has no details step, connect and rent have one', () => {
  assert.deepEqual(steps('script').map((s) => s.id), ['source', 'placement', 'model', 'review']);
  assert.ok(steps('connect').some((s) => s.id === 'details'));
  assert.ok(steps('rent').some((s) => s.id === 'details'));
});

test('node names follow the OpenShell limit', () => {
  assert.equal(nameProblem('gpu-07'), null);
  assert.ok(nameProblem(''));
  assert.ok(nameProblem('Bad_Name'));
  assert.ok(nameProblem('-lead'));
  assert.ok(nameProblem('a'.repeat(43)));
});

test('placement needs a site and a name', () => {
  const s = blankState();
  assert.ok(validate('placement', s));
  s.siteId = 'x';
  s.name = 'gpu-1';
  assert.equal(validate('placement', s), null);
});

test('connect needs an address and a credential', () => {
  const s = blankState();
  s.source = 'connect';
  assert.ok(validate('details', s));
  s.connect.address = '203.0.113.5';
  assert.ok(validate('details', s));
  s.connect.password = 'pw';
  assert.equal(validate('details', s), null);
});

test('connect body carries the password, masked on request', () => {
  const s = blankState();
  Object.assign(s, { source: 'connect', siteId: 's', name: 'n1' });
  Object.assign(s.connect, { address: '1.2.3.4', password: 'secret' });
  assert.equal(bodyFor(s).source.ssh_password, 'secret');
  assert.equal(bodyFor(s, true).source.ssh_password, '********');
  assert.equal(bodyFor(s).source.ssh_private_key, undefined);
  assert.equal(bodyFor(s).model_key, null);
});

test('rent body has numbers and an optional location', () => {
  const s = blankState();
  Object.assign(s, { source: 'rent', siteId: 's', name: 'n1', modelKey: 'm' });
  Object.assign(s.rent, { accountId: 'a', hostType: 'cx32' });
  const b = bodyFor(s);
  assert.equal(b.source.ttl_minutes, 120);
  assert.equal(b.source.location, undefined);
  assert.equal(b.model_key, 'm');
  assert.equal(firstProblem(s), null);
});

test('review lines mention the model only when chosen', () => {
  const s = blankState();
  Object.assign(s, { siteId: 's', name: 'n1' });
  assert.ok(reviewLines(s).some((l) => l.includes('No model')));
  s.modelKey = 'deepseek';
  assert.ok(reviewLines(s).some((l) => l.includes('deepseek')));
});

test('progress follows the log messages', () => {
  const logs = [{ message: 'step provision started' }, { message: 'step join: waiting' }];
  const out = progressStates({ status: 'running' }, logs, false);
  assert.deepEqual(out.map((s) => s.state), ['done', 'active', 'waiting', 'waiting']);
  assert.equal(out.length, 4);
  const failed = progressStates({ status: 'failed' }, logs, true);
  assert.equal(failed[1].state, 'failed');
  const ok = progressStates({ status: 'completed' }, logs, true);
  assert.ok(ok.every((s) => s.state === 'done'));
});

test('only models with an artifact can be deployed', () => {
  const out = deployableModels([{ key: 'a', artifact_ref: 'x' }, { key: 'b' }]);
  assert.deepEqual(out.map((m) => m.key), ['a']);
});
