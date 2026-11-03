// Sorting, filtering, searching, grouping and CSV for the shared table.
//
// Pure functions, so the table's behaviour is tested without a browser. A column
// says how to read a row (`value`), how to draw it (`render`), whether it can be
// sorted, grouped or filtered, and with what kind of filter.

/** Read one cell through a column's `value` accessor, or by its key. */
export function cellValue(row, column) {
  if (!column) return undefined;
  if (typeof column.value === 'function') return column.value(row);
  return row?.[column.key];
}

/** The printable form of a cell: numbers and dates stay readable, null is empty. */
export function cellText(row, column) {
  const value = cellValue(row, column);
  if (value == null) return '';
  if (value instanceof Date) return value.toISOString();
  return String(value);
}

const ISO_DATE =
  /^\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:?\d{2})?)?$/;

/**
 * Turn a value into something the comparator can order.
 *
 * `null` stays `null` so the caller can always put empty cells last. Numbers
 * compare numerically, ISO dates compare as time, everything else compares as
 * lower-case text with numeric awareness.
 */
function comparable(value) {
  if (value == null) return null;
  if (typeof value === 'number') return Number.isNaN(value) ? null : value;
  if (typeof value === 'boolean') return value ? 1 : 0;
  if (value instanceof Date) return value.getTime();
  const text = String(value);
  if (ISO_DATE.test(text)) {
    const asDate = Date.parse(text);
    if (!Number.isNaN(asDate)) return asDate;
  }
  if (text.trim() !== '') {
    const asNumber = Number(text);
    if (!Number.isNaN(asNumber)) return asNumber;
  }
  return text.toLowerCase();
}

function compareValues(a, b, direction) {
  if (a == null && b == null) return 0;
  // Empty cells always sit at the bottom, whichever way the column is sorted.
  if (a == null) return 1;
  if (b == null) return -1;
  if (typeof a === 'string' && typeof b === 'string') {
    return direction * a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
  }
  if (a === b) return 0;
  return direction * (a < b ? -1 : 1);
}

/**
 * Return a sorted copy of `rows`.
 *
 * `sort` is `{key, dir}` where `dir` is `'asc'` or `'desc'`. An unknown key or a
 * missing sort returns the rows unchanged. The input array is never mutated.
 */
export function sortRows(rows, columns, sort) {
  if (!sort?.key) return rows;
  const column = columns.find((candidate) => candidate.key === sort.key);
  if (!column) return rows;
  const direction = sort.dir === 'desc' ? -1 : 1;
  return [...rows].sort((left, right) =>
    compareValues(
      comparable(cellValue(left, column)),
      comparable(cellValue(right, column)),
      direction,
    ),
  );
}

/**
 * Return the rows that match every active filter.
 *
 * A string filter keeps rows whose cell contains it, ignoring case. An array
 * filter keeps rows whose cell equals one of the listed values. Missing or empty
 * filters are ignored; an unknown column key does not exclude anything.
 */
export function filterRows(rows, columns, filters = {}) {
  const active = Object.entries(filters).filter(([, value]) =>
    Array.isArray(value) ? value.length > 0 : value != null && value !== '',
  );
  if (!active.length) return rows;
  return rows.filter((row) =>
    active.every(([key, wanted]) => {
      const column = columns.find((candidate) => candidate.key === key);
      if (!column) return true;
      const text = cellText(row, column).toLowerCase();
      if (Array.isArray(wanted)) {
        const wantedText = wanted.map((entry) => String(entry).toLowerCase());
        return wantedText.includes(text);
      }
      return text.includes(String(wanted).toLowerCase());
    }),
  );
}

/**
 * Return the rows in which any column contains `q`, ignoring case.
 *
 * The search reads the same text the table would print, so a number or a date
 * is searchable without the caller converting it first.
 */
export function searchRows(rows, columns, q) {
  const needle = String(q ?? '')
    .trim()
    .toLowerCase();
  if (!needle) return rows;
  return rows.filter((row) =>
    columns.some((column) => cellText(row, column).toLowerCase().includes(needle)),
  );
}

/**
 * Group rows by one column.
 *
 * Returns `[{key, label, rows, count}]` sorted by count descending, then by
 * label. With no column there is one anonymous group holding every row, which
 * lets the caller use one rendering path for grouped and ungrouped tables.
 */
export function groupRows(rows, column) {
  if (!column) return [{ key: '', label: '', rows, count: rows.length }];
  const groups = new Map();
  for (const row of rows) {
    const value = cellValue(row, column);
    const key = value == null || value === '' ? '(leer)' : String(value);
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(row);
  }
  return [...groups.entries()]
    .map(([key, members]) => ({ key, label: key, rows: members, count: members.length }))
    .sort(
      (left, right) =>
        right.count - left.count ||
        left.label.localeCompare(right.label, undefined, { numeric: true, sensitivity: 'base' }),
    );
}

function csvField(value) {
  if (value == null) return '';
  const text = value instanceof Date ? value.toISOString() : String(value);
  // A field only needs quotes when it would otherwise break the line apart.
  if (/[",\r\n]/.test(text)) return `"${text.replace(/"/g, '""')}"`;
  return text;
}

/**
 * Render rows as CSV: a header line of column labels, then one line per row.
 *
 * Fields containing a quote, a comma or a line break are quoted and inner quotes
 * are doubled, and lines end in CRLF so spreadsheet tools agree on the rows.
 */
export function toCsv(rows, columns) {
  const header = columns.map((column) => csvField(column.label ?? column.key));
  const lines = [header.join(',')];
  for (const row of rows) {
    lines.push(columns.map((column) => csvField(cellValue(row, column))).join(','));
  }
  return lines.join('\r\n');
}
