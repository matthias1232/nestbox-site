// A tiny markdown renderer with no dependency (pattern J10). Wave 0 placeholder:
// package F18 supports headings, lists, code, links, tables and quotes.

const ESCAPES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' };

// Escape text for HTML.
function escape(text) {
  return String(text).replace(/[&<>"]/g, (char) => ESCAPES[char]);
}

// Render markdown to safe HTML; for now the escaped text in one paragraph.
export function render(md) {
  return `<p>${escape(md)}</p>`;
}

// The headings of a document as `[{ id, text, level }]`; empty until F18.
export function headings(md) {
  return [];
}
