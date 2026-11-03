import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeTabs, sshCommand, puttyHint } from './sessions.js';

const mine = [
  { id: 't-1', label: 'Terminal 1', alive: true },
  { id: 't-2', label: 'Build', alive: false },
];

const all = [
  { tab_id: 't-2', title: 'Terminal 2', alive: true, created_by_name: 'ana', viewers: [] },
  { tab_id: 't-9', title: 'Serve', alive: true, created_by_name: 'bo', viewers: [{ user_name: 'bo', kind: 'terminal', view_only: false }] },
];

test('mergeTabs keeps my order and appends the tabs only others have', () => {
  const merged = mergeTabs(mine, all);
  assert.deepEqual(merged.map((tab) => tab.id), ['t-1', 't-2', 't-9']);
  assert.equal(merged[0].mine, true);
  assert.equal(merged[2].mine, false);
});

test('a tab in both lists appears once, keeps my title and carries its viewers', () => {
  const merged = mergeTabs(mine, [
    { tab_id: 't-1', title: 'Somebody else title', alive: true, created_by_name: 'ana', viewers: [{ user_name: 'bo', kind: 'terminal', view_only: true }] },
  ]);
  const tab = merged.find((item) => item.id === 't-1');
  assert.equal(merged.filter((item) => item.id === 't-1').length, 1);
  assert.equal(tab.label, 'Terminal 1');
  // The all-user list is the fresher word on whether the shell still runs.
  assert.equal(tab.alive, true);
  assert.deepEqual(tab.people, [{ name: 'bo', kind: 'terminal', view_only: true }]);
});

test('a shared tab without viewers still names its creator', () => {
  const merged = mergeTabs([], [{ tab_id: 't-7', title: 'Serve', alive: true, created_by_name: 'bo', viewers: [] }]);
  assert.deepEqual(merged[0].people, [{ name: 'bo', kind: 'terminal', view_only: false }]);
});

test('mergeTabs survives a missing or empty answer from either side', () => {
  assert.deepEqual(mergeTabs(undefined, undefined), []);
  assert.deepEqual(mergeTabs(mine, null).map((tab) => tab.id), ['t-1', 't-2']);
  assert.deepEqual(mergeTabs([], all).map((tab) => tab.id), ['t-2', 't-9']);
});

test('sshCommand is the line that attaches one tab', () => {
  assert.equal(sshCommand('10.0.0.5', 22201, 't-1'), 'ssh -t nestbox@10.0.0.5 -p 22201 attach t-1');
});

test('sshCommand takes the sandbox door user when the caller knows it', () => {
  assert.equal(sshCommand('box.example.net', 22201, 't-2', 'ops'), 'ssh -t ops@box.example.net -p 22201 attach t-2');
});

test('a line with a hole in it is not a command', () => {
  assert.equal(sshCommand(null, 22201, 't-1'), null);
  assert.equal(sshCommand('10.0.0.5', 0, 't-1'), null);
  assert.equal(sshCommand('10.0.0.5', 22201, ''), null);
  assert.equal(puttyHint(undefined, 22201, 't-1'), null);
});

test('puttyHint names the three fields PuTTY asks for', () => {
  assert.equal(
    puttyHint('10.0.0.5', 22201, 't-1'),
    'PuTTY: Host 10.0.0.5, Port 22201, Remote command: attach t-1',
  );
});
