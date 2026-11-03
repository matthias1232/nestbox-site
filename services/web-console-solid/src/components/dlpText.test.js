import test from 'node:test';
import assert from 'node:assert/strict';

import { splitDlp } from './dlpText.js';

const iban = {
  action: 'replaced',
  detector: 'iban',
  replacement: '<IBAN_faf7e1c0>',
  masked_value: 'DE**00',
};
const name = {
  action: 'pseudonymized',
  detector: 'name',
  replacement: '<PERSON_0a1b2c3d>',
  masked_value: 'M****',
};

test('plain text is one text part and nothing is recorded', () => {
  const { parts, recorded } = splitDlp('nur Text');
  assert.deepEqual(parts, [{ kind: 'text', text: 'nur Text' }]);
  assert.deepEqual(recorded, []);
});

test('two placeholders become marked parts with their incidents', () => {
  const { parts } = splitDlp('IBAN <IBAN_faf7e1c0> von <PERSON_0a1b2c3d>.', [iban, name]);
  assert.deepEqual(
    parts.map((part) => part.kind),
    ['text', 'replaced', 'text', 'pseudonymized', 'text'],
  );
  assert.equal(parts[1].text, '<IBAN_faf7e1c0>');
  assert.equal(parts[1].incident, iban);
  assert.equal(parts[3].incident, name);
});

test('[REDACTED] is a marked part when an incident names it', () => {
  const redacted = { action: 'redacted', detector: 'secret', replacement: '[REDACTED]' };
  const { parts } = splitDlp('Key: [REDACTED]', [redacted]);
  assert.deepEqual(
    parts.map((part) => part.kind),
    ['text', 'redacted'],
  );
  assert.equal(parts[1].incident, redacted);
});

test('a placeholder without an incident is unknown', () => {
  const { parts } = splitDlp('hier <TOKEN_deadbeef>');
  assert.deepEqual(
    parts.map((part) => part.kind),
    ['text', 'unknown'],
  );
  assert.equal(parts[1].text, '<TOKEN_deadbeef>');
});

test('a blocked incident keeps the blocked kind', () => {
  const blocked = { action: 'blocked', detector: 'api_keys', replacement: '<API_KEYS_12345678>' };
  const { parts } = splitDlp('x <API_KEYS_12345678>', [blocked]);
  assert.equal(parts[1].kind, 'blocked');
});

test('logged findings are returned as recorded and create no part', () => {
  const allowed = {
    action: 'allowed',
    detector: 'iban',
    masked_value: 'DE**00',
  };
  const { parts, recorded } = splitDlp('IBAN DE**00 bleibt', [allowed]);
  assert.deepEqual(parts, [{ kind: 'text', text: 'IBAN DE**00 bleibt' }]);
  assert.deepEqual(recorded, [allowed]);
});
