// Guards on the app's shape that a unit test can check without a DOM.
//
// The console froze on 2026-11-03 because a keyed `Show` received a zero-argument child that
// created `Suspense` and a lazy page: Solid calls such a child inside a tracked effect, and the
// signals the page creation writes re-triggered that effect, re-creating the page forever.
//
// The guard reads every `.jsx` under `src/`, not only `App.jsx`, so a new component cannot bring
// the freeze back. Matching the text `keyed` up to the next `>` is not enough: a tag may carry
// `fallback={<p>…</p>}`, whose elements hold their own `>`, and then the zero-argument child sits
// past them (the spelling of `features/workbox/WorkboxPage.jsx:53`). The scan below walks from
// each `<Show` to the real end of its opening tag, so every spelling is covered.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SRC = dirname(fileURLToPath(import.meta.url));

// Every component under `src/`, relative and sorted, so a failure names a stable path.
const components = readdirSync(SRC, { recursive: true })
  .filter((name) => name.endsWith('.jsx'))
  .sort();

/** The source of a component named the way `components` names it. */
function readComponent(name) {
  return readFileSync(join(SRC, name), 'utf8');
}

/** The 1-based line `index` falls on, for a message that says where to look. */
function lineAt(source, index) {
  return source.slice(0, index).split('\n').length;
}

/**
 * Where the opening tag of the element that starts at `from` ends, and the text of that tag
 * outside its `{…}` expressions.
 *
 * A plain `>` search stops at the `>` of a `fallback={<p>…</p>}` element, which is why this
 * tracks brace depth and string quotes: the tag ends at the first `>` at depth zero, outside a
 * string. The top-level text is what tells a `keyed` attribute from the word in a fallback.
 * Returns `null` for a tag that is never closed, which is a file the parser itself would reject.
 */
function endOfOpeningTag(source, from) {
  const topLevel = [];
  let depth = 0;
  let quote = null;
  for (let i = from; i < source.length; i += 1) {
    const ch = source[i];
    if (quote) {
      if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'" || ch === '`') {
      quote = ch;
      continue;
    }
    if (ch === '{') {
      depth += 1;
      continue;
    }
    if (ch === '}') {
      depth -= 1;
      continue;
    }
    if (depth > 0) continue;
    if (ch === '>') return { end: i, topLevel: topLevel.join('') };
    topLevel.push(ch);
  }
  return null;
}

/**
 * Every keyed `Show` in `source`: the line it starts on, and whether its child function takes
 * the key.
 *
 * The child sits directly after the opening tag. `{(key) => …}` is the only correct form;
 * `{( ) => …}` is called inside a tracked effect, where the signals it writes re-trigger it, and
 * that is the shape that froze the console.
 */
function keyedShows(source) {
  const shows = [];
  for (const match of source.matchAll(/<Show\b/g)) {
    const tag = endOfOpeningTag(source, match.index + match[0].length);
    if (!tag || !/\bkeyed\b/.test(tag.topLevel)) continue;
    const child = /^\s*\{\s*\(\s*\)\s*=>/.exec(source.slice(tag.end + 1));
    shows.push({ line: lineAt(source, match.index), hasKeyParameter: child === null });
  }
  return shows;
}

/** The keyed `Show`s of `source` whose child is a zero-argument function. */
function zeroArgumentKeyedShows(source) {
  return keyedShows(source).filter((show) => !show.hasKeyParameter);
}

test('the guard reads every .jsx under src/, not only App.jsx', () => {
  assert.ok(
    components.length > 100,
    `expected the whole console, found ${components.length} .jsx file(s) under src/`,
  );
  assert.ok(components.includes('App.jsx'), 'App.jsx must be among the files the guard reads');
  assert.ok(
    components.includes(join('features', 'workbox', 'WorkboxPage.jsx')),
    'the nested folders must be read too, not only the top level of src/',
  );
});

test('no keyed Show gets a zero-argument child (it would re-create Suspense forever)', () => {
  const offenders = components.flatMap((name) =>
    zeroArgumentKeyedShows(readComponent(name)).map((show) => `${name}:${show.line}`),
  );
  assert.deepEqual(
    offenders,
    [],
    'a keyed Show must receive the key: {(key) => …}. A zero-argument child runs inside a ' +
      'tracked effect and re-created the page forever (2026-11-03). Fix: ' +
      offenders.join(', '),
  );
});

test('the rule catches every spelling of a zero-argument child and lets the real ones through', () => {
  // A probe string, deliberately not a component file: the shape the guard has to refuse.
  const probes = [
    // The spelling `App.jsx` froze on: the child sits straight after the tag.
    '<Show when={current().id} keyed>{() => <NodePage />}</Show>',
    // The spelling of `WorkboxPage.jsx`, where a fallback element hides the tag's `>`.
    '<Show when={props.id} keyed fallback={<p>None.</p>}>\n  {() => <Page />}\n</Show>',
    // Spaces inside the parentheses are the same function.
    '<Show when={props.id} keyed fallback={<p>None.</p>}>\n  {( ) => <Page />}\n</Show>',
  ];
  for (const probe of probes) {
    assert.equal(
      zeroArgumentKeyedShows(probe).length,
      1,
      `the guard must catch a keyed Show with a zero-argument child: ${probe}`,
    );
  }

  // The legitimate forms in the console today must pass, or the guard would be unusable.
  const legitimate = [
    '<Show when={current().id} keyed>\n  {(id) => <NodePage id={id} />}\n</Show>',
    '<Show when={props.sandboxId} keyed fallback={<p class="muted">No sandbox chosen.</p>}>\n' +
      '      {(id) => <Workbox id={id} />}\n    </Show>',
    '<Show when={session.identity()} fallback={<LoginPage />}>{() => <Shell />}</Show>',
  ];
  for (const source of legitimate) {
    assert.deepEqual(
      zeroArgumentKeyedShows(source),
      [],
      `this keyed Show takes its key or is not keyed at all, so it must pass: ${source}`,
    );
  }
});

test('the guard sees the keyed Shows the console has today', () => {
  // A guard that silently sees nothing would pass forever; these counts fail if it goes blind.
  assert.equal(keyedShows(readComponent('App.jsx')).length, 8);
  assert.equal(
    keyedShows(readComponent(join('features', 'workbox', 'WorkboxPage.jsx'))).length,
    1,
  );
});

test('App.jsx uses Switch/Match for the page, so a menu click changes it', () => {
  const app = readComponent('App.jsx');
  assert.match(app, /<Switch/);
  assert.match(app, /<Match\s+when=/);
});
