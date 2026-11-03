// Tests for the pure alert pop-up mute (U05): the three stored shapes and the expiry rule.
import test from 'node:test';
import assert from 'node:assert/strict';
import { popupsUntil, resolvePopups } from './alertMute.js';

const NOW = 1762171200000; // A fixed moment, so the tests never depend on the clock.

test('pop-ups are on by default and stay on for an unknown value', () => {
  assert.equal(resolvePopups('on', NOW), 'on');
  assert.equal(resolvePopups(undefined, NOW), 'on');
  assert.equal(resolvePopups(null, NOW), 'on');
  assert.equal(resolvePopups('maybe', NOW), 'on');
  assert.equal(resolvePopups({ until: 'later' }, NOW), 'on');
});

test('off means muted until the operator turns them on', () => {
  assert.equal(resolvePopups('off', NOW), 'off');
  assert.equal(resolvePopups('off', NOW + 1000 * 60 * 60 * 24 * 400), 'off');
});

test('a timed mute is off before its moment and on at or after it', () => {
  const until = NOW + 60 * 60 * 1000;
  assert.equal(resolvePopups({ until }, NOW), 'off');
  assert.equal(resolvePopups({ until }, NOW + 60 * 60 * 1000 - 1), 'off');
  assert.equal(resolvePopups({ until }, NOW + 60 * 60 * 1000), 'on');
  assert.equal(resolvePopups({ until }, NOW + 60 * 60 * 1000 + 1), 'on');
});

test('the end of a live timed mute is named, and only while it lasts', () => {
  const until = NOW + 60 * 60 * 1000;
  assert.equal(popupsUntil({ until }, NOW), until);
  assert.equal(popupsUntil({ until }, until), null);
  assert.equal(popupsUntil('off', NOW), null);
  assert.equal(popupsUntil('on', NOW), null);
  assert.equal(popupsUntil({}, NOW), null);
});
