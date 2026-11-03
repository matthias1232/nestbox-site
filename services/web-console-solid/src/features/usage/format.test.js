import test from 'node:test';
import assert from 'node:assert/strict';
import { tokens, money, percent, keyLabel } from './format.js';

test('tokens are compact and a missing number is a dash', () => {
  assert.equal(tokens(950), '950');
  assert.equal(tokens(42_537), '42.5k');
  assert.equal(tokens(3_400_000), '3.40M');
  assert.equal(tokens(undefined), '—');
});

test('a missing price is n/a and never zero', () => {
  assert.equal(money(null), 'n/a');
  assert.equal(money(0), '0.00');
  assert.equal(money(28_050_000), '28.05');
  assert.equal(money(2_500), '0.002500');
});

test('percent and labels', () => {
  assert.equal(percent(0.9438), '94.4 %');
  assert.equal(keyLabel('auth', 'account'), 'Signed-in account');
  assert.equal(keyLabel('conversation', '27a41161-b693-419a'), '27a41161…');
  assert.equal(keyLabel('model', 'claude-opus-5-5'), 'claude-opus-5-5');
});
