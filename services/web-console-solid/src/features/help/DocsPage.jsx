// Docs: /docs and /docs/<slug>. Document list with search on the left, the rendered document
// in the middle, a sticky table of contents on the right, previous/next links below.
import { For, Show, createEffect, createMemo, createSignal } from 'solid-js';
import '../../styles/help.css';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import Empty from '../../components/Empty.jsx';
import { navigate, paths } from '../../routing.js';
import { currentHash, interceptDocLinks, scrollToHeading, useDoc, useIndex } from './useHelp.js';
import { groupByFirstTag, search } from './docsSearch.js';

const REPO = 'https://github.com/matthias1232/nestbox-site/blob/HEAD/services/web-console-solid/public/help';

export default function DocsPage(props) {
  const { index, error: indexError } = useIndex();
  const [query, setQuery] = createSignal('');
  const [target, setTarget] = createSignal(currentHash());
  const { doc, error, loading } = useDoc(() => props.slug);
  let body;

  const groups = createMemo(() => groupByFirstTag(index()));
  const flat = createMemo(() => groups().flatMap((group) => group.docs));
  const hits = createMemo(() => (query().trim() ? search(index(), query()) : []));
  const position = createMemo(() => flat().findIndex((entry) => entry.slug === props.slug));
  const previous = () => (position() > 0 ? flat()[position() - 1] : null);
  const next = () => (position() >= 0 ? flat()[position() + 1] || null : null);
  const title = () => doc()?.meta.title || flat()[position()]?.title || 'Docs';

  // A new document starts at the heading in the address, or at the top.
  createEffect(() => {
    props.slug;
    setTarget(currentHash());
  });
  // Scroll to the wanted heading once the document is on screen.
  createEffect(() => {
    const id = target();
    if (doc() && body) {
      if (id) scrollToHeading(body, id);
      else window.scrollTo?.(0, 0);
    }
  });

  function open(slug, heading) {
    if (slug && slug !== props.slug) navigate(heading ? `${paths.docs(slug)}#${heading}` : paths.docs(slug));
    setTarget(heading || null);
    if (!slug || slug === props.slug) {
      if (heading && body) scrollToHeading(body, heading);
    }
  }

  return (
    <section class="docs-page">
      <Breadcrumb items={props.slug ? [{ label: 'Docs', path: paths.docs() }, { label: title() }] : [{ label: 'Docs' }]} />
      <PageHeader
        title={props.slug ? title() : 'Docs'}
        subtitle={props.slug ? null : 'How the console works, step by step.'}
        actions={
          <Show when={props.slug}>
            <a class="docs-edit" href={`${REPO}/${props.slug}.md`} target="_blank" rel="noopener noreferrer">Edit on GitHub</a>
          </Show>
        }
      />
      <div class="docs-layout">
        <nav class="docs-list" aria-label="Documents">
          <label class="docs-search">
            <span class="eyebrow">Search docs</span>
            <input type="search" value={query()} placeholder="Search titles, tags, headings" onInput={(event) => setQuery(event.currentTarget.value)} />
          </label>
          <Show when={indexError()}>
            <p class="notice">The help index could not be loaded: {indexError()}</p>
          </Show>
          <Show
            when={query().trim()}
            fallback={
              <For each={groups()}>
                {(group) => (
                  <div class="docs-group">
                    <span class="eyebrow">{group.group}</span>
                    <ul>
                      <For each={group.docs}>
                        {(entry) => (
                          <li>
                            <a href={paths.docs(entry.slug)} aria-current={entry.slug === props.slug ? 'page' : undefined} onClick={(event) => { event.preventDefault(); open(entry.slug, null); }}>{entry.title}</a>
                          </li>
                        )}
                      </For>
                    </ul>
                  </div>
                )}
              </For>
            }
          >
            <ul class="docs-hits">
              <For each={hits()} fallback={<li class="muted">Nothing found. Try one word.</li>}>
                {(hit) => (
                  <li>
                    <a href={paths.docs(hit.slug)} onClick={(event) => { event.preventDefault(); open(hit.slug, hit.headingId || null); }}>
                      {hit.title}<Show when={hit.heading}><span class="muted"> › {hit.heading}</span></Show>
                    </a>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </nav>

        <article class="docs-article">
          <Show
            when={props.slug}
            fallback={
              <div class="docs-overview">
                <For each={flat()} fallback={<Empty title="No documents yet" hint="Documents appear here after the help files are added to the build." />}>
                  {(entry) => (
                    <a class="docs-card" href={paths.docs(entry.slug)} onClick={(event) => { event.preventDefault(); open(entry.slug, null); }}>
                      <strong>{entry.title}</strong>
                      <span class="muted small">{entry.summary}</span>
                    </a>
                  )}
                </For>
              </div>
            }
          >
            <Show when={loading() && !doc()}><p class="muted">Loading…</p></Show>
            <Show when={error()}>
              <Empty title="This document could not be loaded" hint={error()} action={<button class="secondary" onClick={() => navigate(paths.docs())}>Back to all documents</button>} />
            </Show>
            <Show when={doc()}>
              <div ref={body} class="md" innerHTML={doc().html} onClick={(event) => interceptDocLinks(event, open)} />
              <nav class="docs-prevnext" aria-label="Previous and next document">
                <Show when={previous()} fallback={<span />}>
                  <a href={paths.docs(previous().slug)} onClick={(event) => { event.preventDefault(); open(previous().slug, null); }}>← {previous().title}</a>
                </Show>
                <Show when={next()}>
                  <a href={paths.docs(next().slug)} onClick={(event) => { event.preventDefault(); open(next().slug, null); }}>{next().title} →</a>
                </Show>
              </nav>
            </Show>
          </Show>
        </article>

        <aside class="docs-toc" aria-label="On this page">
          <Show when={doc()}>
            <span class="eyebrow">On this page</span>
            <ul>
              <For each={doc().headings.filter((heading) => heading.level >= 2)}>
                {(heading) => (
                  <li classList={{ sub: heading.level === 3 }}>
                    <a href={`#${heading.id}`} onClick={(event) => { event.preventDefault(); open(null, heading.id); }}>{heading.text}</a>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </aside>
      </div>
    </section>
  );
}
