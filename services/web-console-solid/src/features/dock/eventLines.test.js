import test from 'node:test';
import assert from 'node:assert/strict';
import { lineFor, pushLine, filterTone } from './eventLines.js';

test('an alert is a bad line that links to the sandbox', () => {
  const line = lineFor('alert', { host: 'x.net', port: 443, sandbox_name: 'box', sandbox_id: 's1' });
  assert.equal(line.tone, 'bad');
  assert.equal(line.text, 'Refused x.net:443 from box');
  assert.equal(line.path, '/sandboxes/s1');
});

test('a failed job is bad and shows progress', () => {
  const line = lineFor('job', { job_id: 'j1', kind: 'node_drain', status: 'failed', completed: 1, total: 4 });
  assert.equal(line.tone, 'bad');
  assert.equal(line.text, 'Job node_drain failed 1/4');
});

test('a running sandbox is ok', () => {
  assert.equal(lineFor('sandbox', { sandbox_id: 'abcdefghij', status: 'running' }).tone, 'ok');
});

test('an unknown event and an empty payload still give a line', () => {
  assert.equal(lineFor('weird', null).text, 'Event weird');
});

test('the list keeps the newest first and is bounded', () => {
  let lines = [];
  for (let i = 0; i < 5; i += 1) lines = pushLine(lines, { tone: 'info', text: String(i) }, 3);
  assert.deepEqual(lines.map((l) => l.text), ['4', '3', '2']);
});

test('the tone filter keeps only that tone', () => {
  const lines = [{ tone: 'bad' }, { tone: 'ok' }];
  assert.equal(filterTone(lines, 'bad').length, 1);
  assert.equal(filterTone(lines, 'all').length, 2);
});
