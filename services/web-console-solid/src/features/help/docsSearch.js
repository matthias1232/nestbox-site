// Searching the help index (contract C17) in the browser: pure, no network.

// Lower-case words of a query; empty words dropped.
function words(q) {
  return String(q).toLowerCase().split(/[^a-z0-9]+/).filter(Boolean);
}

// How well one text matches every word: 0 when a word is missing.
function match(text, terms) {
  const lower = String(text).toLowerCase();
  let score = 0;
  for (const term of terms) {
    if (!lower.includes(term)) return 0;
    score += lower === term ? 3 : lower.startsWith(term) ? 2 : 1;
  }
  return score;
}

// Search an index `[{ slug, title, tags, summary, headings }]`; returns the best hits first as
// `[{ slug, title, heading?, headingId?, score }]`. A heading hit points at the heading.
export function search(index, q) {
  const terms = words(q);
  if (terms.length === 0 || !Array.isArray(index)) return [];
  const hits = [];
  for (const doc of index) {
    const titleScore = match(doc.title, terms) * 5;
    const tagScore = match((doc.tags || []).join(' '), terms) * 3;
    const summaryScore = match(doc.summary || '', terms);
    const docScore = titleScore + tagScore + summaryScore;
    if (docScore > 0) hits.push({ slug: doc.slug, title: doc.title, score: docScore });
    for (const heading of doc.headings || []) {
      const score = match(heading.text, terms) * 2;
      if (score > 0) {
        hits.push({ slug: doc.slug, title: doc.title, heading: heading.text, headingId: heading.id, score });
      }
    }
  }
  return hits.sort((a, b) => b.score - a.score || a.title.localeCompare(b.title));
}

// Group an index by its first tag for the document list: `[{ group, docs }]`, groups sorted.
export function groupByFirstTag(index) {
  const groups = new Map();
  for (const doc of index || []) {
    const group = (doc.tags && doc.tags[0]) || 'other';
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(doc);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([group, docs]) => ({ group, docs: docs.sort((a, b) => a.title.localeCompare(b.title)) }));
}

// Split `slug#heading` into `{ slug, heading }`; either may be null.
export function splitAnchor(anchor) {
  if (!anchor) return { slug: null, heading: null };
  const [slug, heading] = String(anchor).split('#');
  return { slug: slug || null, heading: heading || null };
}
