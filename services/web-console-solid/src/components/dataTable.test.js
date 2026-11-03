import test from 'node:test';
import assert from 'node:assert/strict';

import { filterRows, groupRows, searchRows, sortRows, toCsv } from './dataTable.js';

const columns = [
  { key: 'name', label: 'Name', sortable: true },
  { key: 'size', label: 'Größe', sortable: true },
  { key: 'seen', label: 'Gesehen', sortable: true },
  { key: 'state', label: 'Zustand', filter: 'select' },
];

const rows = [
  { name: 'beta', size: 10, seen: '2026-02-01T10:00:00Z', state: 'running' },
  { name: 'Alpha', size: 9, seen: '2026-01-15T10:00:00Z', state: 'stopped' },
  { name: 'gamma', size: null, seen: null, state: 'running' },
  { name: 'delta', size: 100, seen: '2026-03-01T10:00:00Z', state: 'stopped' },
];

test('numbers sort numerically, not as text', () => {
  const sorted = sortRows(rows, columns, { key: 'size', dir: 'asc' });
  assert.deepEqual(
    sorted.map((row) => row.name),
    ['Alpha', 'beta', 'delta', 'gamma'],
  );
});

test('an ISO date sorts as time, and null stays last in both directions', () => {
  const ascending = sortRows(rows, columns, { key: 'seen', dir: 'asc' });
  assert.deepEqual(
    ascending.map((row) => row.name),
    ['Alpha', 'beta', 'delta', 'gamma'],
  );
  const descending = sortRows(rows, columns, { key: 'seen', dir: 'desc' });
  assert.deepEqual(
    descending.map((row) => row.name),
    ['delta', 'beta', 'Alpha', 'gamma'],
  );
  assert.equal(descending.at(-1).name, 'gamma');
});

test('text sorts without case and a value accessor is used', () => {
  const sorted = sortRows(rows, columns, { key: 'name', dir: 'asc' });
  assert.deepEqual(
    sorted.map((row) => row.name),
    ['Alpha', 'beta', 'delta', 'gamma'],
  );

  const derived = [{ n: 3 }, { n: 1 }, { n: 2 }];
  const byDouble = sortRows(derived, [{ key: 'n', value: (row) => row.n * 2 }], {
    key: 'n',
    dir: 'asc',
  });
  assert.deepEqual(
    byDouble.map((row) => row.n),
    [1, 2, 3],
  );
});

test('sorting does not change the input array', () => {
  const copy = [...rows];
  sortRows(rows, columns, { key: 'name', dir: 'desc' });
  assert.deepEqual(rows, copy);
});

test('a text filter contains, an array filter matches exactly, both ignoring case', () => {
  const contains = filterRows(rows, columns, { name: 'ET' });
  assert.deepEqual(
    contains.map((row) => row.name),
    ['beta'],
  );

  const oneOf = filterRows(rows, columns, { state: ['running'] });
  assert.deepEqual(
    oneOf.map((row) => row.name),
    ['beta', 'gamma'],
  );

  const empty = filterRows(rows, columns, { state: [] });
  assert.equal(empty, rows);
});

test('search looks into every column and ignores case', () => {
  assert.deepEqual(
    searchRows(rows, columns, 'STOPPED').map((row) => row.name),
    ['Alpha', 'delta'],
  );
  assert.deepEqual(
    searchRows(rows, columns, '100').map((row) => row.name),
    ['delta'],
  );
  assert.equal(searchRows(rows, columns, '   '), rows);
});

test('groups are counted, labelled and sorted by size', () => {
  const grouped = groupRows(rows, { key: 'state' });
  assert.deepEqual(
    grouped.map((group) => [group.key, group.count]),
    [
      ['running', 2],
      ['stopped', 2],
    ],
  );
  assert.equal(grouped[0].rows.length, 2);
  assert.equal(grouped[0].label, 'running');
});

test('grouping without a column returns one group', () => {
  const grouped = groupRows(rows, null);
  assert.equal(grouped.length, 1);
  assert.equal(grouped[0].count, rows.length);
  assert.deepEqual(grouped[0].rows, rows);
});

test('empty cells form their own group', () => {
  const grouped = groupRows(rows, { key: 'size' });
  const empty = grouped.find((group) => group.key === '(leer)');
  assert.equal(empty.count, 1);
  assert.equal(empty.rows[0].name, 'gamma');
});

test('CSV quotes commas, quotes and line breaks and doubles inner quotes', () => {
  const csv = toCsv(
    [
      { name: 'plain', note: 'a,b' },
      { name: 'quote', note: 'say "hi"' },
      { name: 'lines', note: 'one\ntwo' },
      { name: null, note: undefined },
    ],
    [
      { key: 'name', label: 'Name' },
      { key: 'note', label: 'Notiz' },
    ],
  );
  assert.equal(
    csv,
    ['Name,Notiz', 'plain,"a,b"', 'quote,"say ""hi"""', 'lines,"one\ntwo"', ','].join('\r\n'),
  );
});

test('CSV writes the column labels as the header line', () => {
  const csv = toCsv([{ size: 3 }], [{ key: 'size', label: 'Größe in MiB' }]);
  assert.equal(csv, 'Größe in MiB\r\n3');
});
