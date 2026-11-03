// The help documents: static files served next to the console (contract C17).
//
// This is the one module besides client.js that calls fetch directly, because the files are
// not part of the API and need neither CSRF nor the environment header.

// The index of every document: `[{ slug, title, tags, summary, headings }]`.
export async function index() {
  const response = await fetch('/help/index.json', { credentials: 'same-origin' });
  if (!response.ok) throw new Error(`Help index failed with ${response.status}`);
  return response.json();
}

// One document as markdown text.
export async function doc(slug) {
  if (!/^[A-Za-z0-9_-]{1,100}$/.test(slug)) throw new Error('Unknown help document');
  const response = await fetch(`/help/${slug}.md`, { credentials: 'same-origin' });
  if (!response.ok) throw new Error(`Help document ${slug} failed with ${response.status}`);
  return response.text();
}
