import test from 'node:test';
import assert from 'node:assert/strict';
import { ADMIN_TABS, resolveTab, subViewFor, tabsFor } from './adminTabs.js';

test('the order starts with summary and live', () => {
  assert.deepEqual(ADMIN_TABS.slice(0, 2).map((tab) => tab.id), ['summary', 'live']);
});

test('a member sees no supervisor and no permissions tab', () => {
  const ids = tabsFor('member').map((tab) => tab.id);
  assert.equal(ids.includes('supervisor'), false);
  assert.equal(ids.includes('permissions'), false);
  assert.equal(ids.includes('summary'), true);
});

test('an operator sees supervisor but not permissions; an admin sees both', () => {
  const operator = tabsFor('operator').map((tab) => tab.id);
  assert.equal(operator.includes('supervisor'), true);
  assert.equal(operator.includes('permissions'), false);
  assert.equal(tabsFor('admin').some((tab) => tab.id === 'permissions'), true);
});

test('old tab ids resolve to the tab that holds them', () => {
  assert.equal(resolveTab('dashboard', 'operator'), 'summary');
  assert.equal(resolveTab('terminal', 'operator'), 'console');
  assert.equal(resolveTab('connections', 'operator'), 'live');
});

test('a hidden or unknown tab falls back to summary', () => {
  assert.equal(resolveTab('supervisor', 'member'), 'summary');
  assert.equal(resolveTab('nonsense', 'admin'), 'summary');
  assert.equal(resolveTab(undefined, 'admin'), 'summary');
});

test('grouped tabs know their sub-view', () => {
  assert.equal(subViewFor('terminal'), 'shell');
  assert.equal(subViewFor('desktop'), 'desktop');
  assert.equal(subViewFor('chats'), 'chats');
  assert.equal(subViewFor('summary'), null);
});

test('every id is unique', () => {
  assert.equal(new Set(ADMIN_TABS.map((tab) => tab.id)).size, ADMIN_TABS.length);
});
