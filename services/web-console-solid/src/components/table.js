// Pure helpers behind SelectTable: sorting and selection.

// A new array of `rows` sorted by `key`; `dir` is 'asc' or 'desc', `type` is 'string' | 'number' | 'date'.
// Missing values sort last in both directions.
export function sortRows(rows, key, dir = 'asc', type = 'string') {
  if (!key) return rows;
  const sign = dir === 'desc' ? -1 : 1;
  const value = (row) => {
    const v = row[key];
    if (v === null || v === undefined || v === '') return null;
    if (type === 'number') return Number(v);
    if (type === 'date') return new Date(v).getTime();
    return String(v).toLowerCase();
  };
  return rows
    .map((row, index) => ({ row, index, v: value(row) }))
    .sort((a, b) => {
      if (a.v === null && b.v === null) return a.index - b.index;
      if (a.v === null) return 1;
      if (b.v === null) return -1;
      const c = a.v < b.v ? -1 : a.v > b.v ? 1 : 0;
      return c !== 0 ? c * sign : a.index - b.index;
    })
    .map((x) => x.row);
}

// The sort state after clicking a header: same key flips the direction, a new key starts ascending.
export function nextSort(sort, key) {
  if (sort && sort.key === key) return { key, dir: sort.dir === 'asc' ? 'desc' : 'asc' };
  return { key, dir: 'asc' };
}

// A new Set with `id` added when absent and removed when present.
export function toggle(set, id) {
  const next = new Set(set);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

// The ids between positions of `a` and `b` in `ids`, both included; an unknown id gives just `b`.
export function rangeSelect(ids, a, b) {
  const i = ids.indexOf(a);
  const j = ids.indexOf(b);
  if (i < 0 || j < 0) return [b];
  return ids.slice(Math.min(i, j), Math.max(i, j) + 1);
}
