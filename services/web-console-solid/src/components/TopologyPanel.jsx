// The side panel of TopologyCanvas for the selected card: facts, connections and the actions the host handles.

import { For, Show, createMemo } from 'solid-js';
import { toneOf } from './topologyLayout.js';

const WORD = { ok: 'ok', warn: 'attention', bad: 'failing', muted: 'unknown' };

export default function TopologyPanel(props) {
  const tone = () => toneOf(props.node.status);
  const names = createMemo(() => new Map((props.nodes || []).map((node) => [node.id, node.label])));
  const incoming = () => (props.edges || []).filter((edge) => edge.to === props.node.id);
  const outgoing = () => (props.edges || []).filter((edge) => edge.from === props.node.id);
  const facts = () => Object.entries(props.node.meta || {}).filter(([, value]) => value !== null && value !== undefined && typeof value !== 'object');
  const refused = () => incoming().some((edge) => edge.refused);
  const isDestination = () => props.node.kind === 'destination';

  return (
    <aside class="topo-panel" aria-label={`Details of ${props.node.label}`}>
      <header>
        <h3 class="mono">{props.node.label}</h3>
        <button class="ghost tiny" aria-label="Close the panel" onClick={() => props.onClose?.()}>Close</button>
      </header>
      <p class="small">
        <span class="muted">{String(props.node.kind).replaceAll('_', ' ')}</span>{' '}
        <span class={`badge ${tone()}`}><i />{props.node.statusLabel || (props.node.status && toneOf(props.node.status) === 'muted' ? props.node.status : WORD[tone()])}</span>
      </p>

      <Show when={facts().length > 0}>
        <table class="topo-facts">
          <tbody>
            <For each={facts()}>
              {([key, value]) => (<tr><th>{key.replaceAll('_', ' ')}</th><td class="mono small">{String(value)}</td></tr>)}
            </For>
          </tbody>
        </table>
      </Show>

      <div class="topo-links">
        <h4>Receives from</h4>
        <Show when={incoming().length} fallback={<p class="muted small">nothing</p>}>
          <ul>
            <For each={incoming()}>
              {(edge) => (
                <li>
                  <button class="link" onClick={() => props.onSelect?.(edge.from)}>{names().get(edge.from) || edge.from}</button>
                  <Show when={edge.refused}> <span class="badge bad"><i />refused</span></Show>
                  <Show when={edge.requests}> <span class="muted small">{edge.requests} req</span></Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <h4>Sends to</h4>
        <Show when={outgoing().length} fallback={<p class="muted small">nothing</p>}>
          <ul>
            <For each={outgoing()}>
              {(edge) => (
                <li>
                  <button class="link" onClick={() => props.onSelect?.(edge.to)}>{names().get(edge.to) || edge.to}</button>
                  <Show when={edge.refused}> <span class="badge bad"><i />refused</span></Show>
                  <Show when={edge.requests}> <span class="muted small">{edge.requests} req</span></Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>

      <div class="actions">
        <Show when={!isDestination()}>
          <button class="secondary tiny" onClick={() => props.onAction?.(props.node.id, 'open')}>Open</button>
          <button class="secondary tiny" onClick={() => props.onAction?.(props.node.id, 'policies')}>Policies</button>
          <button class="secondary tiny" onClick={() => props.onAction?.(props.node.id, 'logs')}>Logs</button>
        </Show>
        <Show when={isDestination() && refused()}>
          <For each={[15, 30, 60]}>
            {(minutes) => (
              <button class="tiny" disabled={props.busy} onClick={() => props.onAction?.(props.node.id, `allow:${minutes}`)}>
                Allow {minutes} min
              </button>
            )}
          </For>
        </Show>
      </div>
    </aside>
  );
}
