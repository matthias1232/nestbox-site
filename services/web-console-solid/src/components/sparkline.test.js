import test from 'node:test';
import assert from 'node:assert/strict';
import { polyline } from './sparkline.js';

test('an empty list gives an empty string', () => {
  assert.equal(polyline([], 100, 20), '');
});

test('the largest value touches the top and zero the bottom', () => {
  assert.equal(polyline([0, 10], 100, 21), '0,20 100,1');
});

test('an explicit max scales the line and clamps above it', () => {
  assert.equal(polyline([5, 50], 10, 21, 10), '0,10.5 10,1');
});

test('one value draws at the horizontal middle', () => {
  assert.equal(polyline([4], 100, 21), '50,1');
});

test('non-numbers are dropped', () => {
  assert.equal(polyline([null, 'x', 0], 10, 21), '5,20');
});
