import test from 'node:test';
import assert from 'node:assert/strict';
import { sortRows, nextSort, toggle, rangeSelect } from './table.js';

const rows = [{ n: 'b', c: 10 }, { n: 'A', c: null }, { n: 'c', c: 2 }];

test('strings sort case-insensitively', () => {
  assert.deepEqual(sortRows(rows, 'n').map((r) => r.n), ['A', 'b', 'c']);
});

test('numbers sort numerically and missing values stay last in both directions', () => {
  assert.deepEqual(sortRows(rows, 'c', 'asc', 'number').map((r) => r.n), ['c', 'b', 'A']);
  assert.deepEqual(sortRows(rows, 'c', 'desc', 'number').map((r) => r.n), ['b', 'c', 'A']);
});

test('the input array is not changed', () => {
  sortRows(rows, 'n', 'desc');
  assert.equal(rows[0].n, 'b');
});

test('nextSort flips the direction on the same key', () => {
  assert.deepEqual(nextSort({ key: 'n', dir: 'asc' }, 'n'), { key: 'n', dir: 'desc' });
  assert.deepEqual(nextSort({ key: 'n', dir: 'desc' }, 'c'), { key: 'c', dir: 'asc' });
});

test('toggle adds and removes without mutating', () => {
  const a = new Set(['x']);
  assert.deepEqual([...toggle(a, 'y')], ['x', 'y']);
  assert.deepEqual([...toggle(a, 'x')], []);
  assert.equal(a.size, 1);
});

test('rangeSelect works in both directions', () => {
  const ids = ['a', 'b', 'c', 'd'];
  assert.deepEqual(rangeSelect(ids, 'b', 'd'), ['b', 'c', 'd']);
  assert.deepEqual(rangeSelect(ids, 'c', 'a'), ['a', 'b', 'c']);
  assert.deepEqual(rangeSelect(ids, 'zz', 'a'), ['a']);
});
