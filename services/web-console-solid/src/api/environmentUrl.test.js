import test from 'node:test';
import assert from 'node:assert/strict';
import { withEnvironment, environmentFromUrl } from './environmentUrl.js';

test('a url without a query gets one', () => {
  assert.equal(withEnvironment('/x', 'e1'), '/x?environment=e1');
});

test('a url with a query keeps its parameters', () => {
  assert.equal(withEnvironment('/x?tab=t1', 'e1'), '/x?tab=t1&environment=e1');
});

test('no environment leaves the url alone', () => {
  assert.equal(withEnvironment('/x', null), '/x');
  assert.equal(withEnvironment('/x', undefined), '/x');
});

test('the environment is read back out of a search string', () => {
  assert.equal(environmentFromUrl('?environment=e1'), 'e1');
  assert.equal(environmentFromUrl('?tab=t1&environment=e1'), 'e1');
  assert.equal(environmentFromUrl(''), null);
});
