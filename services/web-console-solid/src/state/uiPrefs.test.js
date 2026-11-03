import test from 'node:test';
import assert from 'node:assert/strict';
import { DEFAULTS, parse, sanitize, serialize } from './uiPrefs.js';
import { DEFAULTS as DEFAULT_VNC } from '../features/sandboxes/vncOptions.js';

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

test('the alert pop-up choice round-trips in all three shapes', () => {
  for (const alertPopups of ['on', 'off', { until: 1762174800000 }]) {
    const prefs = { ...DEFAULTS, alertPopups };
    assert.deepEqual(parse(serialize(prefs)), prefs);
  }
  assert.equal(DEFAULTS.alertPopups, 'on');
});

test('an unknown alert pop-up value falls back to on', () => {
  assert.equal(sanitize({ alertPopups: 'maybe' }).alertPopups, 'on');
  assert.equal(sanitize({ alertPopups: { until: 'later' } }).alertPopups, 'on');
  assert.equal(sanitize({ alertPopups: true }).alertPopups, 'on');
  assert.equal(sanitize({ alertPopups: null }).alertPopups, 'on');
});

test('out-of-range and unknown values fall back', () => {
  const prefs = sanitize({ sidebarWidth: 5000, dockTab: 'nope', treeView: 'x', consoleMode: { a: 5 }, logPanes: [1, 'ok'] });
  assert.equal(prefs.sidebarWidth, 480);
  assert.equal(prefs.dockTab, 'tasks');
  assert.equal(prefs.treeView, 'infrastructure');
  assert.deepEqual(prefs.consoleMode, {});
  assert.deepEqual(prefs.logPanes, ['ok']);
});

test('the console options of each sandbox round-trip, completed and clamped', () => {
  const prefs = {
    ...DEFAULTS,
    vncOptions: { 'sbx-1': { ...DEFAULT_VNC, scaling: 'none', quality: 9, viewOnly: true } },
  };
  assert.deepEqual(parse(serialize(prefs)), prefs);
  assert.deepEqual(sanitize({ vncOptions: { 'sbx-1': { quality: 99, scaling: 'x' }, 'sbx-2': 'nope' } }).vncOptions['sbx-1'], { ...DEFAULT_VNC, quality: 9 });
  assert.equal(sanitize({ vncOptions: { 'sbx-2': 'nope' } }).vncOptions['sbx-2'], undefined);
  assert.deepEqual(sanitize({ vncOptions: [1, 2] }).vncOptions, {});
});

test('nothing that looks like a token is part of the stored shape', () => {
  assert.ok(!Object.keys(DEFAULTS).some((key) => /token|secret|csrf|password/i.test(key)));
});
