// Guards on the app's shape that a unit test can check without a DOM.
//
// The console froze on 2026-11-03 because a keyed `Show` received a zero-argument child that
// created `Suspense` and a lazy page: Solid calls such a child inside a tracked effect, and the
// signals the page creation writes re-triggered that effect, re-creating the page forever.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const source = readFileSync(new URL('./App.jsx', import.meta.url), 'utf8');

test('a keyed Show never gets a zero-argument child (it would re-create Suspense forever)', () => {
  const bad = /keyed>[\s\S]{0,400}?\{\(\)\s*=>/.test(source);
  assert.equal(bad, false, 'the child of a keyed Show must take the key: {(key) => …}');
});

test('the render function of a keyed Show takes the key', () => {
  assert.match(source, /keyed>\s*\n?\s*\{\([A-Za-z_]+\)\s*=>/);
});

test('App.jsx uses Switch/Match for the page, so a menu click changes it', () => {
  assert.match(source, /<Switch/);
  assert.match(source, /<Match\s+when=/);
});
