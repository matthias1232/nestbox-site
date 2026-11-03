// Validates the help folder and generates public/help/index.json (contract C17).
//
// For every public/help/*.md: the front matter needs title, slug and tags and the slug must
// equal the file name; every `[text](/docs/<slug>)` link must name an existing document and
// every image must live under /illustrations/. The index lists slug, title, tags, summary
// (first paragraph) and the ## and ### headings. Exits 1 with one line per problem.
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { headings, parseFrontMatter } from '../src/components/markdown.js';

const dir = new URL('../public/help/', import.meta.url);
const distDir = new URL('../dist/help/', import.meta.url);

const names = (await readdir(dir)).filter((name) => name.endsWith('.md')).sort();
const documents = [];
const problems = [];

for (const name of names) {
  const text = await readFile(new URL(name, dir), 'utf8');
  const { meta, body } = parseFrontMatter(text);
  for (const field of ['title', 'slug', 'tags']) {
    const value = meta[field];
    if (!value || (Array.isArray(value) && value.length === 0)) {
      problems.push(`${name}: front matter field "${field}" is missing`);
    }
  }
  if (meta.slug && meta.slug !== name.replace(/\.md$/, '')) {
    problems.push(`${name}: slug "${meta.slug}" must equal the file name`);
  }
  documents.push({ name, meta, body });
}

const slugs = new Set(documents.map((document) => document.meta.slug).filter(Boolean));

for (const { name, meta, body } of documents) {
  // Links and images outside fenced code blocks.
  const prose = body.replace(/```[\s\S]*?```/g, '').replace(/`[^`\n]*`/g, '');
  for (const match of prose.matchAll(/(!?)\[[^\]]*\]\(([^)\s]+)\)/g)) {
    const [, bang, target] = match;
    if (bang) {
      if (!target.startsWith('/illustrations/')) problems.push(`${name}: image ${target} is not under /illustrations/`);
    } else if (target.startsWith('/docs/')) {
      const slug = target.slice('/docs/'.length).split('#')[0];
      if (!slugs.has(slug)) problems.push(`${name}: link ${target} points to a document that does not exist`);
    }
  }
  const ids = new Set(headings(body).map((heading) => heading.id));
  for (const anchor of meta.anchors || []) {
    if (!ids.has(anchor)) problems.push(`${name}: anchor "${anchor}" has no heading`);
  }
}

// The first paragraph after the title, cut to 200 characters.
function summary(body) {
  for (const block of body.split(/\n\s*\n/)) {
    const line = block.trim();
    if (line && !/^(#|```|\||>|-|\d+\.|!\[)/.test(line)) {
      return line.replace(/\s+/g, ' ').replace(/[*`]/g, '').slice(0, 200);
    }
  }
  return '';
}

if (problems.length > 0) {
  console.error(`check-help: ${problems.length} problem(s)`);
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exit(1);
}

const index = documents.map(({ meta, body }) => ({
  slug: meta.slug,
  title: meta.title,
  tags: meta.tags,
  summary: summary(body),
  headings: headings(body)
    .filter((heading) => heading.level >= 2)
    .map(({ id, text }) => ({ id, text })),
}));
const json = `${JSON.stringify(index, null, 2)}\n`;
await writeFile(new URL('index.json', dir), json);
// The build copies public/ before this script runs, so refresh the built copy too.
if (existsSync(new URL('../dist/', import.meta.url))) {
  await mkdir(distDir, { recursive: true });
  await writeFile(new URL('index.json', distDir), json);
}
console.log(`check-help: ${index.length} document(s) indexed`);
