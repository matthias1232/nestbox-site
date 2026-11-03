import test from 'node:test';
import assert from 'node:assert/strict';
import { ageText, tunnelLabel, tunnelTone } from './tunnels.js';

test('a handshake a few seconds old reads in seconds', () => {
  assert.equal(ageText(12), '12 s');
  assert.equal(ageText(59), '59 s');
  assert.equal(ageText(0), '0 s');
});

test('minutes and hours keep the unit readable', () => {
  assert.equal(ageText(60), '1 min');
  assert.equal(ageText(599), '9 min');
  assert.equal(ageText(3600), '1 h');
  assert.equal(ageText(3900), '1 h 05 min');
});

test('days round down to whole days', () => {
  assert.equal(ageText(2 * 86400), '2 d');
  assert.equal(ageText(2 * 86400 + 3600), '2 d');
});

test('an age nobody sent prints nothing', () => {
  assert.equal(ageText(null), '');
  assert.equal(ageText(undefined), '');
  assert.equal(ageText('nonsense'), '');
});

test('a negative age is clamped rather than shown as a negative time', () => {
  assert.equal(ageText(-5), '0 s');
});

test('a state reads with its age, as the badge shows it', () => {
  assert.equal(tunnelLabel('up', 12), 'up · 12 s');
  assert.equal(tunnelLabel('stale', 400), 'stale · 6 min');
  assert.equal(tunnelLabel('down', null), 'down');
  assert.equal(tunnelLabel('UP', 3), 'up · 3 s');
});

test('a state nobody sent reads as unknown', () => {
  assert.equal(tunnelLabel(null, null), 'unknown');
  assert.equal(tunnelLabel('', undefined), 'unknown');
});

test('each tunnel state has its badge colour', () => {
  assert.equal(tunnelTone('up'), 'ok');
  assert.equal(tunnelTone('stale'), 'warn');
  assert.equal(tunnelTone('down'), 'bad');
  assert.equal(tunnelTone('unknown'), 'muted');
  assert.equal(tunnelTone('something_new'), 'muted');
  assert.equal(tunnelTone(null), 'muted');
});
