import test from 'node:test';
import assert from 'node:assert/strict';

import { createTicker } from './liveTicker.js';

test('the ticker keeps at most max events, newest first', () => {
  const ticker = createTicker({ max: 2 });
  ticker.upsert({ id: 'a', ts: '2026-01-01T10:00:00Z' });
  ticker.upsert({ id: 'b', ts: '2026-01-01T10:00:01Z' });
  ticker.upsert({ id: 'c', ts: '2026-01-01T10:00:02Z' });
  assert.deepEqual(
    ticker.items().map((item) => item.id),
    ['c', 'b'],
  );
});

test('an event with the same key is merged, not added twice', () => {
  const ticker = createTicker();
  ticker.upsert({ id: 'a', ts: 1, state: 'open', detail: 'first' });
  const merged = ticker.upsert({ id: 'a', ts: 2, state: 'closed' });
  assert.equal(ticker.items().length, 1);
  assert.deepEqual(merged, { id: 'a', ts: 2, state: 'closed', detail: 'first' });
  assert.deepEqual(ticker.items(), [merged]);
});

test('items are sorted by ts descending even when inserted out of order', () => {
  const ticker = createTicker();
  ticker.upsert({ id: 'middle', ts: 1_700_000_100 });
  ticker.upsert({ id: 'oldest', ts: 1_700_000_000 });
  ticker.upsert({ id: 'newest', ts: 1_700_000_200 });
  assert.deepEqual(
    ticker.items().map((item) => item.id),
    ['newest', 'middle', 'oldest'],
  );
});

test('a custom key keeps events apart and groups updates', () => {
  const ticker = createTicker();
  const byDestination = (item) => item.destination;
  ticker.upsert({ destination: 'api.example', ts: 1, count: 1 }, byDestination);
  ticker.upsert({ destination: 'api.example', ts: 2, count: 2 }, byDestination);
  ticker.upsert({ destination: 'other.example', ts: 3, count: 1 }, byDestination);
  assert.equal(ticker.items().length, 2);
  assert.equal(ticker.items()[1].count, 2);
});

test('remove drops one event by key and clear drops everything', () => {
  const ticker = createTicker();
  ticker.upsert({ id: 'a', ts: 1 });
  ticker.upsert({ id: 'b', ts: 2 });
  ticker.remove('a');
  assert.deepEqual(
    ticker.items().map((item) => item.id),
    ['b'],
  );
  ticker.clear();
  assert.deepEqual(ticker.items(), []);
});
