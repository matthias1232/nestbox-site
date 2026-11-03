import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULTS, parse, sanitize, serialize } from './uiPrefs.js';

test('empty or broken storage gives the defaults', () => {
  assert.deepEqual(parse(null), { ...DEFAULTS });
  assert.deepEqual(parse('{not json'), { ...DEFAULTS });
  assert.deepEqual(parse('42'), { ...DEFAULTS });
});

test('preferences round-trip', () => {
  const prefs = { ...DEFAULTS, sidebarCollapsed: true, dockOpen: true, dockTab: 'alerts', treeView: 'models',
    consoleMode: { abc: 'desktop' }, logPanes: ['all'], toursDismissed: ['first-run'] };
  assert.deepEqual(parse(serialize(prefs)), prefs);
});

test('out-of-range and unknown values fall back', () => {
  const prefs = sanitize({ sidebarWidth: 5000, dockTab: 'nope', treeView: 'x', consoleMode: { a: 5 }, logPanes: [1, 'ok'] });
  assert.equal(prefs.sidebarWidth, 480);
  assert.equal(prefs.dockTab, 'tasks');
  assert.equal(prefs.treeView, 'infrastructure');
  assert.deepEqual(prefs.consoleMode, {});
  assert.deepEqual(prefs.logPanes, ['ok']);
});

test('nothing that looks like a token is part of the stored shape', () => {
  assert.ok(!Object.keys(DEFAULTS).some((key) => /token|secret|csrf|password/i.test(key)));
});
