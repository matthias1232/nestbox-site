import test from 'node:test';
import assert from 'node:assert/strict';
import { createMatcher, isTypingTarget, parseCombo, stepMatches } from './shortcuts.js';

test('mod+k parses to one step with the modifier', () => {
  assert.deepEqual(parseCombo('mod+k'), [{ key: 'k', mod: true, shift: false, alt: false }]);
});

test('g h parses to two steps', () => {
  assert.equal(parseCombo('g h').length, 2);
});

test('mod is Ctrl off the Mac and Cmd on it', () => {
  const [step] = parseCombo('mod+k');
  assert.equal(stepMatches(step, { key: 'k', ctrlKey: true }, false), true);
  assert.equal(stepMatches(step, { key: 'k', metaKey: true }, false), false);
  assert.equal(stepMatches(step, { key: 'k', metaKey: true }, true), true);
  assert.equal(stepMatches(step, { key: 'k' }, false), false);
});

test('a plain key does not fire with Ctrl held', () => {
  const [step] = parseCombo('g');
  assert.equal(stepMatches(step, { key: 'g' }), true);
  assert.equal(stepMatches(step, { key: 'g', ctrlKey: true }), false);
});

test('? matches although it needs Shift', () => {
  const [step] = parseCombo('?');
  assert.equal(stepMatches(step, { key: '?', shiftKey: true }), true);
});

test('a two-key sequence completes inside the window and not outside it', () => {
  const matcher = createMatcher('g h');
  assert.equal(matcher.feed({ key: 'g' }, 1000), false);
  assert.equal(matcher.feed({ key: 'h' }, 1500), true);
  assert.equal(matcher.feed({ key: 'g' }, 2000), false);
  assert.equal(matcher.feed({ key: 'h' }, 3000), false);
});

test('a wrong key resets the sequence', () => {
  const matcher = createMatcher('g h');
  matcher.feed({ key: 'g' }, 0);
  assert.equal(matcher.feed({ key: 'x' }, 10), false);
  assert.equal(matcher.feed({ key: 'h' }, 20), false);
});

test('inputs and the terminal are typing targets', () => {
  assert.equal(isTypingTarget({ tagName: 'INPUT' }), true);
  assert.equal(isTypingTarget({ tagName: 'DIV', closest: (s) => (s === '.xterm' ? {} : null) }), true);
  assert.equal(isTypingTarget({ tagName: 'DIV', closest: () => null }), false);
  assert.equal(isTypingTarget(null), false);
});
