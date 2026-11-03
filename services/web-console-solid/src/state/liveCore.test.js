import test from 'node:test';
import assert from 'node:assert/strict';
import { createRegistry, nextDelay, payloadFor } from './liveCore.js';

test('the reconnect delay doubles from 1 s and stops at 30 s', () => {
  const steps = [];
  let delay = 0;
  for (let i = 0; i < 8; i += 1) {
    delay = nextDelay(delay);
    steps.push(delay);
  }
  assert.deepEqual(steps, [1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
});

test('a handler receives events until it is stopped', () => {
  const registry = createRegistry();
  const seen = [];
  const stop = registry.on('job', (p) => seen.push(p));
  registry.emit('job', 1);
  stop();
  registry.emit('job', 2);
  assert.deepEqual(seen, [1]);
  assert.equal(registry.count('job'), 0);
});

test('a throwing handler does not stop the others', () => {
  const registry = createRegistry();
  const seen = [];
  registry.on('x', () => { throw new Error('boom'); });
  registry.on('x', (p) => seen.push(p));
  registry.emit('x', 'ok');
  assert.deepEqual(seen, ['ok']);
});

test('an alert event hands over the alert view', () => {
  assert.deepEqual(payloadFor('alert', { type: 'alert', alert: { host: 'a' } }), { host: 'a' });
  assert.deepEqual(payloadFor('job', { type: 'job_progress' }), { type: 'job_progress' });
});
