// One turn: the question, the answer, the tool calls in between, the findings on
// the text, and the two ways to take it away.
//
// The findings of a turn live behind `chats.turn(id)`; they are fetched when the
// turn first scrolls into view, at most four at a time (`chatFindings.js`),
// because a thread of two hundred turns must not open two hundred requests.
import { For, Show, createSignal, onCleanup, onMount } from 'solid-js';
import DlpText from '../../components/DlpText.jsx';
import RouteChain from '../../components/RouteChain.jsx';
import DlpFindingDrawer from '../privacy/DlpFindingDrawer.jsx';
import { turnDetails } from './chatFindings.js';
import { copyText } from './chatsClipboard.js';
import {
  formatClock,
  modelLabel,
  normaliseToolCalls,
  providerLabel,
  toolText,
  tokenLine,
} from './chatsFormat.js';

export default function ChatTurn(props) {
  const turn = () => props.turn || {};
  const [incidents, setIncidents] = createSignal(turn().incidents || []);
  const [stored, setStored] = createSignal(null);
  const [toolsOpen, setToolsOpen] = createSignal(false);
  const [copied, setCopied] = createSignal('');
  const [finding, setFinding] = createSignal(null);
  let element;

  const blocked = () => turn().delivered === false;
  const pending = () => Boolean(turn().pending);
  const answer = () => turn().assistant_text || stored()?.turn?.assistant_text || '';
  const tools = () => normaliseToolCalls(turn().tool_calls ?? stored()?.turn?.tool_calls);
  const route = () => turn().route || stored()?.turn?.route || [];

  async function loadFindings() {
    const id = turn().id;
    // A running turn has no stored row yet; it gets one when `chat_done`
    // replaces it, and this component then sees a real id.
    if (!id || String(id).startsWith('live:')) return;
    const detail = await turnDetails.loadTurn(id);
    if (!detail) return;
    setStored(detail);
    if (Array.isArray(detail.incidents)) setIncidents(detail.incidents);
  }

  onMount(() => {
    if (typeof IntersectionObserver === 'undefined') {
      void loadFindings();
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        void loadFindings();
      },
      { rootMargin: '120px' },
    );
    observer.observe(element);
    onCleanup(() => observer.disconnect());
  });

  async function copy(which, text) {
    const ok = await copyText(text);
    if (!ok) return;
    setCopied(which);
    setTimeout(() => setCopied(''), 1400);
  }

  function openFirstFinding() {
    const first = incidents()[0];
    if (first) setFinding(first);
  }

  return (
    <article
      class="chat-turn"
      classList={{ 'is-pending': pending(), 'is-blocked': blocked() }}
      ref={element}
    >
      <Show when={turn().user_text}>
        <div class="chat-bubble user">
          <p class="chat-text">
            <DlpText
              text={turn().user_text}
              incidents={incidents()}
              onFinding={(incident) => setFinding(incident)}
            />
          </p>
          <div class="chat-meta">
            <span class="chat-clock">{formatClock(turn().ts)}</span>
            <span class="chat-chip">{modelLabel(turn())}</span>
            <button
              type="button"
              class="ghost tiny chat-copy"
              title="Frage kopieren (maskiert)"
              onClick={() => copy('user', turn().user_text)}
            >
              {copied() === 'user' ? 'kopiert' : 'Kopieren'}
            </button>
          </div>
        </div>
      </Show>

      <div
        class="chat-bubble model"
        classList={{ blocked: blocked(), pending: pending() }}
        onClick={blocked() ? openFirstFinding : undefined}
        role={blocked() ? 'button' : undefined}
        title={blocked() ? 'Blockiert: das erste Vorkommnis ansehen' : undefined}
      >
        <Show when={blocked()}>
          <p class="chat-blocked-banner">
            Blockiert – nicht an {providerLabel(turn())}/{modelLabel(turn())} zugestellt
          </p>
        </Show>

        <Show when={answer()} fallback={
          <Show when={pending()}>
            <span class="chat-typing" aria-label="Antwort entsteht">
              <i /><i /><i />
            </span>
          </Show>
        }>
          <p class="chat-text chat-grow">
            <DlpText
              text={answer()}
              incidents={incidents()}
              blocked={blocked()}
              onFinding={(incident) => setFinding(incident)}
            />
          </p>
        </Show>

        <Show when={tools().length}>
          <div class="chat-tools">
            <button
              type="button"
              class="chat-tools-toggle"
              aria-expanded={toolsOpen()}
              onClick={() => setToolsOpen((open) => !open)}
            >
              {toolsOpen() ? '▾' : '▸'} {tools().length}{' '}
              {tools().length === 1 ? 'Tool-Aufruf' : 'Tool-Aufrufe'}
            </button>
            <Show when={toolsOpen()}>
              <pre class="chat-tool-block">
                <For each={tools()}>
                  {(call) => <code class="chat-tool-line">{toolText(call)}</code>}
                </For>
              </pre>
            </Show>
          </div>
        </Show>

        <Show when={blocked()}>
          <RouteChain route={route()} compact />
        </Show>

        <div class="chat-meta">
          <Show when={pending()}>
            <span class="chat-live-dot" title="The answer is still growing">running</span>
          </Show>
          <span class="chat-tokens">{tokenLine(turn())}</span>
          <span class="chat-clock">{formatClock(turn().ended_at || turn().ts)}</span>
          <span class="chat-chip">{modelLabel(turn())}</span>
          <button
            type="button"
            class="ghost tiny chat-copy"
            title="Antwort kopieren (maskiert)"
            onClick={(event) => {
              event.stopPropagation();
              copy('model', answer());
            }}
          >
            {copied() === 'model' ? 'kopiert' : 'Kopieren'}
          </button>
        </div>
      </div>

      <DlpFindingDrawer
        open={Boolean(finding())}
        incident={finding()}
        onClose={() => setFinding(null)}
      />
    </article>
  );
}
