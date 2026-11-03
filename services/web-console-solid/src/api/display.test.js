import test from 'node:test';
import assert from 'node:assert/strict';
import { displayBody } from './display.js';

const layout = {
  mode: 'separate',
  monitors: [
    { id: 'm1', name: 'Monitor 1', width: 1920, height: 1080, x: 0, y: 0, primary: true },
  ],
};

test('a write without a stamp sends only the layout', () => {
  assert.deepEqual(displayBody(layout), { layout });
});

test('a write with a stamp sends it so the server can refuse a stale one', () => {
  assert.deepEqual(displayBody(layout, '2026-11-03T10:00:00Z'), {
    layout,
    expected_applied_at: '2026-11-03T10:00:00Z',
  });
});

test('an empty stamp is no stamp', () => {
  assert.deepEqual(displayBody(layout, ''), { layout });
  assert.deepEqual(displayBody(layout, null), { layout });
});
