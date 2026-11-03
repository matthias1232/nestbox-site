// One thread of model calls, oldest first, as a messenger would show it.
//
// Contract (other files build against it): `turns` is an array of chat turns, a
// running turn carries `pending: true`. `title` is optional.
//
// The order is decided here, not by the caller: a thread reads from the oldest
// question to the running answer, whatever order the API paged it in. The
// viewport follows a growing answer only while the reader is at the bottom;
// otherwise a count appears and scrolling stays where the reader put it.
import { For, Show, createEffect, createSignal, onMount } from 'solid-js';
import ChatThreadHead from './ChatThreadHead.jsx';
import ChatTurn from './ChatTurn.jsx';
import { sortTurns } from './chatsMerge.js';

/** How close to the bottom still counts as "following along". */
const STICKY_PX = 64;

export default function ChatThread(props) {
  const ordered = () => sortTurns(props.turns || [], 'asc');
  const [stuck, setStuck] = createSignal(true);
  const [missed, setMissed] = createSignal(0);
  let box;
  let lastRevision = '';

  // A cheap revision: how many turns, how long the last answer is, whether it
  // still runs. Any of the three means the thread grew.
  const revision = () => {
    const list = ordered();
    const last = list[list.length - 1];
    return `${list.length}:${last?.assistant_text?.length || 0}:${last?.pending ? 1 : 0}`;
  };

  function atBottom() {
    if (!box) return true;
    return box.scrollHeight - box.scrollTop - box.clientHeight < STICKY_PX;
  }

  function onScroll() {
    const bottom = atBottom();
    setStuck(bottom);
    if (bottom) setMissed(0);
  }

  function toBottom() {
    if (box) box.scrollTop = box.scrollHeight;
    setStuck(true);
    setMissed(0);
  }

  onMount(() => {
    if (box) box.scrollTop = box.scrollHeight;
  });

  createEffect(() => {
    const now = revision();
    // A new array with the same content is not a new message; only a real
    // change may raise the "new messages" count.
    if (now === lastRevision) return;
    lastRevision = now;
    if (!box) return;
    if (stuck()) {
      // The list renders after this effect, so the height is still the old one.
      queueMicrotask(() => {
        if (box) box.scrollTop = box.scrollHeight;
      });
    } else {
      setMissed((count) => count + 1);
    }
  });

  return (
    <div class="chat-thread">
      <Show when={props.title}><h3 class="chat-title">{props.title}</h3></Show>
      <Show when={ordered().length}>
        <ChatThreadHead turns={ordered()} />
      </Show>
      <div class="chat-scroll" ref={box} onScroll={onScroll}>
        <For each={ordered()} fallback={<p class="muted small">No questions yet.</p>}>
          {(turn) => <ChatTurn turn={turn} />}
        </For>
      </div>
      <Show when={missed() > 0}>
        <button type="button" class="chat-jump" onClick={toBottom}>
          New messages ↓
        </button>
      </Show>
    </div>
  );
}
