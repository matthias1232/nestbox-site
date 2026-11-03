import test from 'node:test';
import assert from 'node:assert/strict';
import { MAX_IN_FLIGHT, createFindingsQueue } from './findingsQueue.js';

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

// A loader whose answers the test hands out one at a time, so it can see what
// the queue does while four requests are still open.
function controllable() {
  const calls = [];
  let running = 0;
  let peak = 0;
  const load = (id) =>
    new Promise((resolve, reject) => {
      running += 1;
      peak = Math.max(peak, running);
      const call = { id, settled: false };
      call.resolve = (value) => {
        if (call.settled) return;
        call.settled = true;
        running -= 1;
        resolve(value);
      };
      call.reject = (error) => {
        if (call.settled) return;
        call.settled = true;
        running -= 1;
        reject(error);
      };
      calls.push(call);
    });
  return { load, calls, peak: () => peak, running: () => running };
}

// Answer everything still open until the queue is quiet.
async function drain(fake, queue) {
  for (let round = 0; round < 20; round += 1) {
    const open = fake.calls.filter((call) => !call.settled);
    if (!open.length && queue.active() === 0 && queue.waiting() === 0) return;
    for (const call of open) call.resolve({ incidents: [] });
    await flush();
  }
}

test('four details are in flight, the rest waits', async () => {
  const fake = controllable();
  const queue = createFindingsQueue({ load: fake.load });
  const promises = ['t1', 't2', 't3', 't4', 't5', 't6'].map((id) => queue.loadTurn(id));
  await flush();

  assert.equal(fake.calls.length, MAX_IN_FLIGHT);
  assert.equal(queue.active(), MAX_IN_FLIGHT);
  assert.equal(queue.waiting(), 2);

  fake.calls[0].resolve({ incidents: [{ detector: 'iban' }] });
  await flush();
  assert.equal(fake.calls.length, MAX_IN_FLIGHT + 1);
  assert.equal(queue.waiting(), 1);

  await drain(fake, queue);
  const results = await Promise.all(promises);
  assert.equal(results.length, 6);
  assert.equal(fake.peak() <= MAX_IN_FLIGHT, true);
});

test('a turn that was read once is not read again', async () => {
  const fake = controllable();
  const queue = createFindingsQueue({ load: fake.load });
  const first = queue.loadTurn('t1');
  await flush();
  fake.calls[0].resolve({ incidents: [{ detector: 'iban' }] });
  assert.deepEqual(await first, { incidents: [{ detector: 'iban' }] });
  assert.equal(fake.calls.length, 1);

  const second = await queue.loadTurn('t1');
  assert.deepEqual(second, { incidents: [{ detector: 'iban' }] });
  assert.equal(fake.calls.length, 1);
  assert.deepEqual(queue.cached('t1'), { incidents: [{ detector: 'iban' }] });
});

test('two views asking for the same turn share one request', async () => {
  const fake = controllable();
  const queue = createFindingsQueue({ load: fake.load });
  const left = queue.loadTurn('t1');
  const right = queue.loadTurn('t1');
  await flush();
  assert.equal(fake.calls.length, 1);
  fake.calls[0].resolve({ incidents: [] });
  assert.deepEqual(await left, { incidents: [] });
  assert.deepEqual(await right, { incidents: [] });
});

test('a failed detail is a missing detail, not a broken turn', async () => {
  const fake = controllable();
  const queue = createFindingsQueue({ load: fake.load });
  const first = queue.loadTurn('t1');
  const second = queue.loadTurn('t2');
  await flush();
  fake.calls[0].reject(new Error('boom'));
  await flush();

  assert.equal(await first, null);
  assert.equal(queue.active(), 1);
  assert.equal(fake.calls.length, 2);

  fake.calls[1].resolve({ incidents: [] });
  assert.deepEqual(await second, { incidents: [] });
});

test('no id is no request', async () => {
  const fake = controllable();
  const queue = createFindingsQueue({ load: fake.load });
  assert.equal(await queue.loadTurn(null), null);
  await flush();
  assert.equal(fake.calls.length, 0);
});
