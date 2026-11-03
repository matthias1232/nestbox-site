import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

// The decision lives in a pure helper next to the Solid wrapper. Read only that part, because
// the module itself imports solid-js and these tests run without the app's dependencies.
const source = readFileSync(new URL('./Poll.js', import.meta.url), 'utf8');
const pure = source.slice(source.indexOf('export function pollActive'), source.indexOf('export function usePolling'));
const { pollActive } = new Function(`${pure.replaceAll('export ', '')}; return { pollActive };`)();

test('a visible, unpaused tab polls', () => {
  assert.equal(pollActive({ hidden: false, paused: false }), true);
});

test('a parked poll and a hidden tab both stay quiet', () => {
  assert.equal(pollActive({ hidden: false, paused: true }), false);
  assert.equal(pollActive({ hidden: true, paused: false }), false);
});

test('a hidden tab does not poll even when unpaused, so the pause switch is not the only gate', () => {
  assert.equal(pollActive({ hidden: true, paused: true }), false);
});

test('coming back into view resumes a poll the user did not pause, and only then', () => {
  const paused = { hidden: false, paused: true };
  assert.equal(pollActive({ ...paused, hidden: true }), false);
  assert.equal(pollActive({ ...paused, hidden: false }), false);
  assert.equal(pollActive({ hidden: false, paused: false }), true);
});

test('the timer follows the helper and the visibility listener is removed on cleanup', () => {
  assert.match(source, /pollActive\(\{ hidden: hidden\(\), paused: paused\(\) \}\)/);
  assert.match(source, /document\.addEventListener\('visibilitychange', onVisibility\)/);
  assert.match(source, /document\.removeEventListener\('visibilitychange', onVisibility\)/);
});
