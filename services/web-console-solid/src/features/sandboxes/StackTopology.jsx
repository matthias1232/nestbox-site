// The stack diagram: what this sandbox works with, and how it is wired, from
// you to the internet.
//
// One read of `GET /sandboxes/{id}/stack-topology`, repeated every few seconds
// while "Live" is on. The server decides what is in the stack and what state
// each piece is in; this file draws it in bands (one per layer) and answers
// "what is this card, what touches it, and what did it print?".
//
// Every card is a button. Selecting one lights its connections, and the panel
// below shows what it is, its facts, how to open it (when it has an address) and
// its log, read through the same viewer wherever it comes from.

import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import * as sandboxApi from '../../api/sandboxes.js';
import * as systemApi from '../../api/system.js';
import LogViewer from '../../components/LogViewer.jsx';
import TopologyCanvas from '../../components/TopologyCanvas.jsx';
import { fromStack } from '../../components/topologyLayout.js';
import { drawnEdges, headline, logLoader, neighbours, tone } from './stackLayout.js';
import '../../styles/stack-topology.css';

const POLL_MS = 5000;
const WINDOWS = ['1m', '5m', '15m', '1h'];
const STATUS_WORD = { ok: 'ok', warn: 'attention', error: 'failing', idle: 'stopped', unknown: 'unknown', absent: 'not there' };

async function copy(text) {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    /* the clipboard is not available on plain http; the text is selectable */
  }
}

export default function StackTopology(props) {
  const [topology, setTopology] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [live, setLive] = createSignal(true);
  const [span, setSpan] = createSignal('5m');
  const [selected, setSelected] = createSignal(null);
  const [readAt, setReadAt] = createSignal(null);

  let timer = null;

  async function load() {
    const id = props.sandbox?.id;
    if (!id) return;
    try {
      const answer = await sandboxApi.stackTopology(id, span());
      setTopology(answer);
      setReadAt(new Date());
      setError(null);
      if (!selected() || !answer.nodes.some((node) => node.id === selected())) {
        const own = answer.nodes.find((node) => node.sandbox_id === id && node.kind === 'sandbox');
        setSelected(own ? own.id : answer.nodes[0]?.id || null);
      }
    } catch (failure) {
      setError(failure.message || 'The stack could not be read.');
    }
  }

  function schedule() {
    if (timer) clearInterval(timer);
    timer = live() ? setInterval(load, POLL_MS) : null;
  }

  onMount(() => {
    load();
    schedule();
  });
  onCleanup(() => timer && clearInterval(timer));

  createEffect(() => {
    live();
    schedule();
  });
  createEffect(() => {
    span();
    load();
  });
  createEffect(() => {
    props.revision;
    load();
  });

  // The canvas data: merged edges (worst status wins) in the shape of contract C15.
  const drawing = createMemo(() => {
    const data = topology();
    return fromStack(data ? { ...data, edges: drawnEdges(data) } : null);
  });
  const current = createMemo(() => topology()?.nodes.find((node) => node.id === selected()) || null);
  const around = createMemo(() => (selected() ? neighbours(topology(), selected()) : { incoming: [], outgoing: [] }));

  // The canvas asks for the log panel (or the address) of a card; the detail below shows both.
  function onCanvasAction(id, action) {
    setSelected(id);
    if (action === 'open') {
      const url = topology()?.nodes.find((node) => node.id === id)?.url;
      if (url) window.open(url, '_blank', 'noopener,noreferrer');
    }
  }

  const loader = () => logLoader(current(), { sandboxLogs: (id) => sandboxApi.logs(id, 400), componentLogs: (id) => systemApi.componentLogs(id, 400) });

  return (
    <section class="stack">
      <div class="controls stack-head">
        <button class={live() ? 'primary tiny' : 'secondary tiny'} onClick={() => setLive(!live())}>
          {live() ? 'Live' : 'Paused'}
        </button>
        <button class="secondary tiny" onClick={load}>Refresh</button>
        <label class="small muted">
          traffic window{' '}
          <select value={span()} onChange={(event) => setSpan(event.currentTarget.value)}>
            <For each={WINDOWS}>{(item) => <option value={item}>{item}</option>}</For>
          </select>
        </label>
        <span class="muted small">
          <Show when={readAt()} fallback="reading…">updated {readAt().toLocaleTimeString()}</Show>
        </span>
      </div>

      <Show when={error()}><p class="error">{error()}</p></Show>

      <Show when={topology()}>
        {(data) => (
          <>
            <p class="stack-headline">
              <span class={`dot ${data().counts.error ? 'bad' : data().counts.warn ? 'warn' : 'ok'}`} />
              {headline(data())}
              <span class="muted small">
                {' '}{data().nodes.length} pieces, {data().edges.length} connections
              </span>
            </p>
            <Show when={data().problems.length > 0}>
              <ul class={data().problems.some((text) => /block|deny|denied|fail/i.test(text)) ? 'stack-problems stack-problems--alert' : 'stack-problems'}>
                <For each={data().problems.slice(0, 6)}>{(text) => <li>{text}</li>}</For>
              </ul>
            </Show>

            <TopologyCanvas
              label="Stack topology"
              nodes={drawing().nodes}
              edges={drawing().edges}
              layers={drawing().layers}
              selectedId={selected()}
              onSelect={(id) => id && setSelected(id)}
              onAction={onCanvasAction}
              panel={false}
              height={460}
            />

            <Show when={current()}>
              {(item) => (
                <div class="stack-detail">
                  <header>
                    <h3 class="mono">{item().name}</h3>
                    <span class={`badge ${tone(item().status)}`}><i />{STATUS_WORD[item().status] || item().status}</span>
                    <span class="muted small">{item().kind.replace('_', ' ')} · {item().state}</span>
                  </header>
                  <p>{item().description}</p>

                  <Show when={item().url}>
                    <p class="controls">
                      <a class="button primary tiny" href={item().url} target="_blank" rel="noopener noreferrer">Open</a>
                      <span class="mono small">{item().url}</span>
                      <button class="tiny" onClick={() => copy(item().url)}>Copy</button>
                    </p>
                  </Show>

                  <Show when={item().facts.length > 0}>
                    <dl class="stack-facts">
                      <For each={item().facts}>
                        {(entry) => (<><dt>{entry.label}</dt><dd class="mono small">{entry.value}</dd></>)}
                      </For>
                    </dl>
                  </Show>

                  <div class="stack-links">
                    <div>
                      <h4>Receives from</h4>
                      <Show when={around().incoming.length} fallback={<p class="muted small">nothing</p>}>
                        <ul>
                          <For each={around().incoming}>
                            {(link) => (
                              <li><button class="link" onClick={() => setSelected(link.other)}>{link.otherName}</button> <span class="muted small">{link.label || link.kind}</span></li>
                            )}
                          </For>
                        </ul>
                      </Show>
                    </div>
                    <div>
                      <h4>Sends to</h4>
                      <Show when={around().outgoing.length} fallback={<p class="muted small">nothing</p>}>
                        <ul>
                          <For each={around().outgoing}>
                            {(link) => (
                              <li><button class="link" onClick={() => setSelected(link.other)}>{link.otherName}</button> <span class="muted small">{link.label || link.kind}</span></li>
                            )}
                          </For>
                        </ul>
                      </Show>
                    </div>
                  </div>

                  <Show
                    when={loader()}
                    fallback={<p class="muted small">This piece has no log of its own.</p>}
                  >
                    <h4>Log</h4>
                    <LogViewer
                      source={item().id}
                      filter={item().logs?.filter || ''}
                      load={() => loader()()}
                    />
                  </Show>
                </div>
              )}
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
