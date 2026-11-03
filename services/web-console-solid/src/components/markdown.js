// A tiny markdown renderer with no dependency (pattern J10).
//
// Supports headings (#-###) with slug ids, paragraphs, **bold**, *em*, `code`, fenced code
// blocks, - lists, 1. lists, [text](url), ![alt](src), tables, > quotes and horizontal rules.
// All text is escaped first; links may only point to http(s)://, / or #.

const ESCAPES = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' };

// Escape text for HTML.
function escape(text) {
  return String(text).replace(/[&<>"]/g, (char) => ESCAPES[char]);
}

// Turn a heading text into an id: lower case, words joined by `-`.
export function slugify(text) {
  return (
    String(text)
      .toLowerCase()
      .replace(/[`*_]/g, '')
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'section'
  );
}

// True when a link or image target is allowed (checked on the raw, unescaped target).
function safeTarget(url, allowRelativeOnly = false) {
  if (url.startsWith('//')) return false;
  if (url.startsWith('/')) return true;
  if (!allowRelativeOnly && url.startsWith('#')) return true;
  return /^https?:\/\//i.test(url);
}

// Inline markup of one already-trimmed text: code spans, images, links, bold, emphasis.
function inline(text) {
  const spans = [];
  // Code spans are lifted out first so their content is never read as markup.
  let work = String(text).replace(/`([^`]+)`/g, (_, code) => {
    spans.push(`<code>${escape(code)}</code>`);
    return `\u0000${spans.length - 1}\u0000`;
  });
  work = escape(work);
  work = work.replace(/!\[([^\]]*)\]\(([^)\s]+)\)/g, (match, alt, src) => {
    const raw = src.replace(/&amp;/g, '&');
    return safeTarget(raw, true) ? `<img src="${src}" alt="${alt}" loading="lazy">` : match;
  });
  work = work.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (match, label, href) => {
    const raw = href.replace(/&amp;/g, '&');
    if (!safeTarget(raw)) return match;
    const external = /^https?:/i.test(raw) ? ' target="_blank" rel="noopener noreferrer"' : '';
    return `<a href="${href}"${external}>${label}</a>`;
  });
  work = work.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
  work = work.replace(/(^|[^*])\*([^*\s][^*]*)\*(?!\*)/g, '$1<em>$2</em>');
  return work.replace(/\u0000(\d+)\u0000/g, (_, index) => spans[Number(index)]);
}

// Split one table row into trimmed cells.
function cells(line) {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim());
}

const FENCE = /^```\s*([A-Za-z0-9_-]*)\s*$/;
const HEADING = /^(#{1,3})\s+(.+?)\s*#*\s*$/;
const RULE = /^(-{3,}|\*{3,})\s*$/;
const BULLET = /^\s*[-*]\s+(.*)$/;
const NUMBERED = /^\s*\d+\.\s+(.*)$/;
const TABLE_SEPARATOR = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

// Hands out unique heading ids, adding `-2`, `-3` to repeats.
function idMaker() {
  const seen = new Map();
  return (text) => {
    const base = slugify(text);
    const count = (seen.get(base) || 0) + 1;
    seen.set(base, count);
    return count === 1 ? base : `${base}-${count}`;
  };
}

// Render markdown to safe HTML.
export function render(md) {
  const lines = String(md).replace(/\r\n?/g, '\n').split('\n');
  const makeId = idMaker();
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === '') {
      i += 1;
      continue;
    }
    const fence = FENCE.exec(line.trim());
    if (fence) {
      const body = [];
      i += 1;
      while (i < lines.length && !/^```\s*$/.test(lines[i].trim())) {
        body.push(lines[i]);
        i += 1;
      }
      i += 1;
      const lang = fence[1] ? ` class="lang-${escape(fence[1])}"` : '';
      out.push(`<pre><code${lang}>${escape(body.join('\n'))}</code></pre>`);
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      const level = heading[1].length;
      out.push(`<h${level} id="${makeId(heading[2])}">${inline(heading[2])}</h${level}>`);
      i += 1;
      continue;
    }
    if (RULE.test(line.trim())) {
      out.push('<hr>');
      i += 1;
      continue;
    }
    if (line.trim().startsWith('|') && i + 1 < lines.length && TABLE_SEPARATOR.test(lines[i + 1])) {
      const head = cells(line);
      i += 2;
      const rows = [];
      while (i < lines.length && lines[i].trim().startsWith('|')) {
        rows.push(cells(lines[i]));
        i += 1;
      }
      const th = head.map((cell) => `<th>${inline(cell)}</th>`).join('');
      const tr = rows
        .map((row) => `<tr>${row.map((cell) => `<td>${inline(cell)}</td>`).join('')}</tr>`)
        .join('');
      out.push(`<table><thead><tr>${th}</tr></thead><tbody>${tr}</tbody></table>`);
      continue;
    }
    if (/^\s*>/.test(line)) {
      const body = [];
      while (i < lines.length && /^\s*>/.test(lines[i])) {
        body.push(lines[i].replace(/^\s*>\s?/, ''));
        i += 1;
      }
      out.push(`<blockquote>${render(body.join('\n'))}</blockquote>`);
      continue;
    }
    const listPattern = BULLET.test(line) ? BULLET : NUMBERED.test(line) ? NUMBERED : null;
    if (listPattern) {
      const tag = listPattern === BULLET ? 'ul' : 'ol';
      const items = [];
      while (i < lines.length && listPattern.test(lines[i])) {
        items.push(listPattern.exec(lines[i])[1]);
        i += 1;
      }
      out.push(`<${tag}>${items.map((item) => `<li>${inline(item)}</li>`).join('')}</${tag}>`);
      continue;
    }
    const para = [];
    while (
      i < lines.length &&
      lines[i].trim() !== '' &&
      !FENCE.test(lines[i].trim()) &&
      !HEADING.test(lines[i]) &&
      !BULLET.test(lines[i]) &&
      !NUMBERED.test(lines[i]) &&
      !/^\s*>/.test(lines[i]) &&
      !lines[i].trim().startsWith('|')
    ) {
      para.push(lines[i].trim());
      i += 1;
    }
    out.push(`<p>${inline(para.join(' '))}</p>`);
  }
  return out.join('\n');
}

// The headings of a document as `[{ id, text, level }]`; ids match the ones `render` emits.
export function headings(md) {
  const makeId = idMaker();
  const found = [];
  let inFence = false;
  for (const line of String(md).replace(/\r\n?/g, '\n').split('\n')) {
    if (FENCE.test(line.trim()) || /^```\s*$/.test(line.trim())) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const match = HEADING.exec(line);
    if (match) {
      const text = match[2].replace(/[`*]/g, '');
      found.push({ id: makeId(match[2]), text, level: match[1].length });
    }
  }
  return found;
}

// Split a document into its front matter fields and its body: `{ meta, body }`.
// `tags` becomes an array; `anchors: [a, b]` becomes an array; other values stay text.
export function parseFrontMatter(md) {
  const text = String(md).replace(/\r\n?/g, '\n');
  const match = /^---\n([\s\S]*?)\n---\n?/.exec(text);
  if (!match) return { meta: {}, body: text };
  const meta = {};
  for (const line of match[1].split('\n')) {
    const pair = /^([A-Za-z_]+):\s*(.*)$/.exec(line);
    if (!pair) continue;
    let value = pair[2].trim();
    if (value.startsWith('[') && value.endsWith(']')) value = value.slice(1, -1);
    meta[pair[1]] = pair[1] === 'tags' || pair[1] === 'anchors'
      ? value.split(',').map((part) => part.trim()).filter(Boolean)
      : value;
  }
  return { meta, body: text.slice(match[0].length) };
}
