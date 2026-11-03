import test from 'node:test';
import assert from 'node:assert/strict';
import { arcPath, fraction, isHot } from './gauge.js';

test('an empty gauge draws no arc', () => {
  assert.equal(arcPath(0, 100, 10), '');
});

test('a half gauge sweeps 135 degrees and uses the small-arc flag', () => {
  const d = arcPath(50, 100, 10);
  assert.match(d, /^M 2\.93 17\.07 A 10 10 0 0 1 10 0$/);
});

test('a full gauge ends where 405 degrees ends and uses the large-arc flag', () => {
  const d = arcPath(100, 100, 10);
  assert.match(d, / 0 1 1 17\.07 17\.07$/);
});

test('a value above max clamps to full', () => {
  assert.equal(arcPath(500, 100, 10), arcPath(100, 100, 10));
  assert.equal(fraction(500, 100), 1);
});

test('bad input reads as zero and 90 percent is hot', () => {
  assert.equal(fraction(5, 0), 0);
  assert.equal(fraction(-3, 10), 0);
  assert.equal(fraction('x', 10), 0);
  assert.equal(isHot(90, 100), true);
  assert.equal(isHot(89, 100), false);
});
