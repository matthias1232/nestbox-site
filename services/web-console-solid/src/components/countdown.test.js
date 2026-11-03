import test from 'node:test';
import assert from 'node:assert/strict';

import { formatRemaining, remainingMs } from './countdown.js';

test('a minute and five seconds reads as 1:05', () => {
  assert.equal(formatRemaining(65 * 1000), '1:05');
});

test('an hour and five minutes reads as 1 h 05 min', () => {
  assert.equal(formatRemaining(3_900_000), '1 h 05 min');
});

test('zero and everything below it has expired', () => {
  assert.equal(formatRemaining(0), 'expired');
  assert.equal(formatRemaining(-5000), 'expired');
});

test('no end time has no remaining time', () => {
  assert.equal(remainingMs(null, 0), null);
});

test('the remaining time is the distance to the end time', () => {
  assert.equal(remainingMs('1970-01-01T00:01:00Z', 0), 60_000);
});
