import test from 'node:test';
import assert from 'node:assert/strict';
import { labelFor, toneFor, summaryLine, parseTargets, failedTargets, durationText, hasTargets } from './jobLabels.js';

test('the three Control Center kinds have names', () => {
  assert.equal(labelFor('model_bulk_install'), 'Deploy model on nodes');
  assert.equal(labelFor('node_quick_deploy'), 'Add node');
  assert.equal(labelFor('node_drain'), 'Drain node');
  assert.equal(labelFor('sandbox_bulk_action'), 'Bulk action on sandboxes');
});

test('an unknown kind is made readable', () => {
  assert.equal(labelFor('some_new_kind'), 'Some new kind');
});

test('tones follow the status', () => {
  assert.equal(toneFor('failed'), 'bad');
  assert.equal(toneFor('succeeded'), 'ok');
  assert.equal(toneFor('partial'), 'warn');
  assert.equal(toneFor('cancelled'), 'muted');
});

test('the summary line counts done and failed', () => {
  assert.equal(summaryLine({ total: 20, completed: 12, failed: 1 }), '12 of 20 done, 1 failed');
  assert.equal(summaryLine({ total: 4, completed: 4, failed: 0 }), '4 of 4 done');
  assert.equal(summaryLine({ total: 0, status: 'running' }), 'Running');
});

test('target results come from JSON log lines only', () => {
  const rows = [
    { message: '{"sandbox_id":"a","ok":true,"message":"started"}' },
    { message: '{"node":"gpu-01","ok":false,"message":"too small"}' },
    { message: 'plain line' },
    { message: '{broken' },
  ];
  const targets = parseTargets(rows);
  assert.equal(targets.length, 2);
  assert.deepEqual(failedTargets(rows), ['gpu-01']);
});

test('duration is formatted', () => {
  const job = { started_at: '2026-01-01T00:00:00Z', finished_at: '2026-01-01T00:02:05Z' };
  assert.equal(durationText(job), '2 min 05 s');
  assert.equal(durationText({}), '');
});

test('fan-out kinds have targets', () => {
  assert.equal(hasTargets('node_drain'), true);
  assert.equal(hasTargets('catalog_sync'), false);
});
