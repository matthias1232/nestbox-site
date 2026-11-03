// "My sandboxes": the cards a person can click to enter their workbox.
//
// A member reaches few boxes and gets one list. An operator reaches every box in
// the data center and gets the list split, because "mine" and "someone else's"
// are different pieces of work even when they look the same.

import { For, Show, createMemo, onMount } from 'solid-js';
import Empty from '../../components/Empty.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import { boxPath, navigate } from '../../routing.js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { splitByOwner, useMyBoxes } from './useWorkbox.js';
import '../../styles/workbox.css';

/**
 * The list of sandboxes the caller can open.
 */
export default function MyBoxesPage() {
  const session = useSession();
  const ui = useUi();
  const boxes = useMyBoxes();
  onMount(() => boxes.load());

  // `null` means one plain list; the split only exists for operators and admins.
  const grouped = createMemo(() => splitByOwner(boxes.items(), session.identity()));

  const viewers = (id) => boxes.summaryOf(id)?.viewers || [];
  const allowAll = (id) => Boolean(boxes.summaryOf(id)?.allow_all);

  // One card: status, node, the loud badge, and one-click Open / Shell / Desktop.
  // The buttons are real buttons beside the title link, not nested in a button.
  function card(item) {
    const connected = () => viewers(item.id).length;
    const open = (mode) => {
      if (mode) ui.console.setMode(item.id, mode);
      navigate(boxPath(item.id));
    };
    return (
      <article class="wb-card wb-box">
        <span class="wb-box-head">
          <a
            href={boxPath(item.id)}
            class="wb-box-title"
            onClick={(event) => {
              event.preventDefault();
              open(null);
            }}
          >
            <strong>{item.name}</strong>
          </a>
          <StatusBadge status={item.status} />
        </span>
        <span class="wb-box-meta muted small">
          <span class="mono">{item.pack_key}</span>
          <span>·</span>
          <span>{item.node_name || 'local node'}</span>
        </span>
        <span class="wb-box-flags">
          <Show when={allowAll(item.id)}>
            <span class="badge bad"><i />ALL ALLOWED</span>
          </Show>
          <Show when={connected() > 0}>
            <span class="badge muted">
              <i />{connected()} connected
            </span>
          </Show>
        </span>
        <span class="wb-box-actions">
          <button class="primary tiny" onClick={() => open(null)}>Open</button>
          <button class="secondary tiny" onClick={() => open('shell')}>Shell</button>
          <button class="secondary tiny" onClick={() => open('desktop')}>Desktop</button>
        </span>
      </article>
    );
  }

  function grid(list) {
    return (
      <div class="wb-boxes">
        <For each={list}>{(item) => card(item)}</For>
      </div>
    );
  }

  return (
    <section class="page wb-page">
      <header class="wb-header">
        <h1>My sandboxes</h1>
      </header>

      <Show when={boxes.error()}>
        <div class="wb-error">
          <p class="error">{boxes.error()}</p>
          <button class="secondary tiny" onClick={() => boxes.load()}>Retry</button>
        </div>
      </Show>

      <Show when={boxes.loading() && boxes.items().length === 0}>
        <p class="muted">Loading…</p>
      </Show>

      <Show when={!boxes.loading() && boxes.items().length === 0 && !boxes.error()}>
        <Empty
          title="No sandboxes yet."
          hint="A sandbox is created from a pack or a recipe. An operator can make one for you."
        />
      </Show>

      <Show when={boxes.items().length > 0}>
        <Show when={grouped()} fallback={grid(boxes.items())}>
          <h2>Mine</h2>
          <Show
            when={grouped().mine.length > 0}
            fallback={<p class="muted small">None of the visible sandboxes is yours.</p>}
          >
            {grid(grouped().mine)}
          </Show>
          <h2>Shared and others</h2>
          <Show
            when={grouped().others.length > 0}
            fallback={<p class="muted small">Nothing else is visible in this data center.</p>}
          >
            {grid(grouped().others)}
          </Show>
        </Show>
      </Show>
    </section>
  );
}
