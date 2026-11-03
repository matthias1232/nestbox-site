import test from 'node:test';
import assert from 'node:assert/strict';
import { modes, pickMode, nextFont, sshLines } from './consoleFrame.js';

test('a normal sandbox offers shell, desktop and ssh', () => {
  assert.deepEqual(modes({}, 'member'), ['shell', 'desktop', 'ssh']);
});

test('a sandbox without a desktop hides it and a viewer gets no ssh', () => {
  assert.deepEqual(modes({ has_desktop: false }, 'viewer'), ['shell']);
});

test('pickMode falls back to the first offered mode', () => {
  assert.equal(pickMode('desktop', ['shell']), 'shell');
  assert.equal(pickMode('ssh', ['shell', 'ssh']), 'ssh');
});

test('the font stays between 11 and 20', () => {
  assert.equal(nextFont(11, -1), 11);
  assert.equal(nextFont(20, 1), 20);
  assert.equal(nextFont(13, 1), 14);
  assert.equal(nextFont(undefined, -1), 12);
});

test('ssh lines are built from host and port, or null', () => {
  assert.deepEqual(sshLines(null), { command: null, config: null });
  const lines = sshLines({ host: 'h', ports: [{ purpose: 'ssh', public_port: 2201 }] });
  assert.equal(lines.command, 'ssh nestbox@h -p 2201');
  assert.match(lines.config, /Port 2201/);
});
