import test from 'node:test';
import assert from 'node:assert/strict';
import { downloadText, exportFilename, toJson, toMarkdown } from './chatExport.js';

const turn = {
  id: 't1',
  ts: '2026-10-01T10:00:00Z',
  user_text: 'bitte <IBAN_ab12cd34> prüfen',
  assistant_text: 'Der Platzhalter <IBAN_ab12cd34> steht für eine IBAN.',
  served_model: 'claude-sonnet',
  provider: 'anthropic',
  delivered: true,
  tool_calls: [{ name: 'read_file', input: { path: 'a.txt' } }],
  input_tokens: 1204,
  output_tokens: 318,
};

test('markdown names the asker, the time and the model', () => {
  const markdown = toMarkdown([turn]);
  assert.match(markdown, /^\*\*User\*\* \(\d{4}-\d{2}-\d{2} \d{2}:\d{2}, claude-sonnet\): bitte <IBAN_ab12cd34> prüfen$/m);
  assert.match(markdown, /^\*\*Model\*\*: Der Platzhalter <IBAN_ab12cd34> steht für eine IBAN\.$/m);
});

test('markdown writes tool calls as a code block', () => {
  const markdown = toMarkdown([turn]);
  assert.match(markdown, /```tool\nread_file \{"path":"a\.txt"\}\n```/);
});

test('markdown keeps the masked text and never a value', () => {
  const markdown = toMarkdown([turn]);
  assert.equal(markdown.includes('<IBAN_ab12cd34>'), true);
  assert.equal(/(<IBAN_ab12cd34>)\s*→/.test(markdown), false);
});

test('a blocked turn says it was not delivered', () => {
  const blocked = {
    ...turn,
    assistant_text: '',
    delivered: false,
    dlp: 'blocked',
  };
  const markdown = toMarkdown([blocked]);
  assert.match(markdown, /\*\*Model\*\* \(blocked, not delivered\):/);
  assert.match(markdown, /> Not delivered to anthropic\/claude-sonnet\./);
});

test('a running turn is marked as unfinished, not as empty', () => {
  const running = { ...turn, assistant_text: '', pending: true };
  assert.match(toMarkdown([running]), /\*\*Model\*\*: …/);
});

test('two turns are separated, not glued', () => {
  const markdown = toMarkdown([turn, turn]);
  assert.equal(markdown.split('\n\n---\n\n').length, 2);
  assert.equal(toMarkdown([]), '');
});

test('json is the same thread as objects', () => {
  const parsed = JSON.parse(toJson([turn]));
  assert.equal(parsed.length, 1);
  assert.equal(parsed[0].id, 't1');
  assert.equal(parsed[0].model, 'claude-sonnet');
  assert.equal(parsed[0].user_text, 'bitte <IBAN_ab12cd34> prüfen');
  assert.equal(parsed[0].delivered, true);
  assert.deepEqual(parsed[0].tokens, { input: 1204, output: 318, cache_read: 0, cache_write: 0 });
  assert.deepEqual(parsed[0].tool_calls, [{ name: 'read_file', input: { path: 'a.txt' } }]);
});

test('the json export is stable for an empty thread', () => {
  assert.equal(toJson([]), '[]');
});

test('the filename is time-ordered and names the console', () => {
  const at = new Date(2026, 9, 1, 9, 5);
  assert.equal(exportFilename('md', at), 'nestbox-chat-20261001-0905.md');
  assert.equal(exportFilename('json', at), 'nestbox-chat-20261001-0905.json');
});

test('a download without a document reports failure instead of throwing', () => {
  assert.equal(downloadText('# x', 'x.md'), false);
});
