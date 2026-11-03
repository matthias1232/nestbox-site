import test from 'node:test';
import assert from 'node:assert/strict';
import { render, headings, slugify, parseFrontMatter } from './markdown.js';

test('headings get slug ids and levels', () => {
  const html = render('# One\n## Pick the model\n### Deep');
  assert.match(html, /<h1 id="one">One<\/h1>/);
  assert.match(html, /<h2 id="pick-the-model">Pick the model<\/h2>/);
  assert.match(html, /<h3 id="deep">Deep<\/h3>/);
});

test('headings() agrees with the ids render() emits, including repeats and fenced code', () => {
  const md = '## A\n```\n## not a heading\n```\n## A';
  assert.deepEqual(headings(md), [
    { id: 'a', text: 'A', level: 2 },
    { id: 'a-2', text: 'A', level: 2 },
  ]);
  assert.match(render(md), /id="a-2"/);
});

test('paragraphs join their lines', () => {
  assert.equal(render('one\ntwo'), '<p>one two</p>');
});

test('bold, emphasis and inline code', () => {
  assert.equal(render('**b** *e* `c*d*`'), '<p><strong>b</strong> <em>e</em> <code>c*d*</code></p>');
});

test('fenced code is escaped and kept verbatim', () => {
  const html = render('```sh\necho "<hi>" | cat\n```');
  assert.equal(html, '<pre><code class="lang-sh">echo &quot;&lt;hi&gt;&quot; | cat</code></pre>');
});

test('bullet and numbered lists', () => {
  assert.equal(render('- a\n- b'), '<ul><li>a</li><li>b</li></ul>');
  assert.equal(render('1. a\n2. b'), '<ol><li>a</li><li>b</li></ol>');
});

test('links and images with allowed targets', () => {
  assert.match(render('[docs](/docs/x#y)'), /<a href="\/docs\/x#y">docs<\/a>/);
  assert.match(render('[up](#top)'), /<a href="#top">up<\/a>/);
  assert.match(render('[w](https://a.example/p?x=1&y=2)'), /href="https:\/\/a\.example\/p\?x=1&amp;y=2" target="_blank" rel="noopener noreferrer"/);
  assert.match(render('![alt](/illustrations/a.webp)'), /<img src="\/illustrations\/a\.webp" alt="alt" loading="lazy">/);
});

test('tables', () => {
  const html = render('| A | B |\n|---|---|\n| 1 | `2` |');
  assert.equal(html, '<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td><code>2</code></td></tr></tbody></table>');
});

test('quotes and rules', () => {
  assert.equal(render('> note **x**'), '<blockquote><p>note <strong>x</strong></p></blockquote>');
  assert.equal(render('---'), '<hr>');
});

test('a script tag is escaped', () => {
  const html = render('<script>alert(1)</script> **x**');
  assert.ok(!html.includes('<script'));
  assert.match(html, /&lt;script&gt;/);
});

test('javascript: and protocol-relative targets are not turned into links', () => {
  assert.ok(!render('[x](javascript:alert(1))').includes('<a '));
  assert.ok(!render('[x](//evil.example)').includes('<a '));
  assert.ok(!render('![x](javascript:alert(1))').includes('<img'));
  assert.ok(!render('![x](#frag)').includes('<img'));
});

test('a quote in an attribute cannot break out', () => {
  const html = render('[x](/a"onclick="alert(1))');
  assert.ok(!html.includes('"onclick'));
});

test('slugify', () => {
  assert.equal(slugify('Make a cluster!'), 'make-a-cluster');
  assert.equal(slugify('!!!'), 'section');
});

test('front matter is parsed and removed from the body', () => {
  const { meta, body } = parseFrontMatter('---\ntitle: T\nslug: t\ntags: a, b\nanchors: [x, y]\n---\n# H');
  assert.equal(meta.title, 'T');
  assert.deepEqual(meta.tags, ['a', 'b']);
  assert.deepEqual(meta.anchors, ['x', 'y']);
  assert.equal(body, '# H');
});
