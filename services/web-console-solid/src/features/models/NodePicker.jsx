// The node step of the wizard: nodes grouped data center › site, a checkbox and a fit badge each.
import { For, Show, createSignal } from 'solid-js';
import { groupNodes, selectFitting, selectBySite, selectByDc, summary } from './bulkDeploy.js';

const WORDS = { fits: 'fits', tight: 'tight', no: 'skipped' };
const TONES = { fits: 'ok', tight: 'warn', no: 'bad' };

// Props: cands (candidateNodes rows), selected (Set of node ids), onChange(Set), text, onText(string).
export default function NodePicker(props) {
  const [lastSite, setLastSite] = createSignal(null);
  const groups = () => groupNodes(props.cands);
  const flip = (id) => {
    const next = new Set(props.selected);
    if (next.has(id)) next.delete(id); else next.add(id);
    props.onChange(next);
  };
  const addAll = (ids) => props.onChange(new Set([...props.selected, ...ids]));
  return (
    <div class="node-picker">
      <div class="node-picker-tools">
        <input
          type="search"
          placeholder="Filter nodes…"
          aria-label="Filter nodes"
          value={props.text}
          onInput={(event) => props.onText(event.currentTarget.value)}
        />
        <button type="button" class="secondary tiny" onClick={() => props.onChange(selectFitting(props.cands))}>All that fit</button>
        <button type="button" class="ghost tiny" onClick={() => props.onChange(new Set())}>Clear</button>
      </div>
      <p class="muted small" aria-live="polite">{summary(props.cands, props.selected)}</p>
      <Show when={props.cands.length} fallback={<p class="muted">No node matches. Clear the filter or join a node first.</p>}>
        <For each={groups()}>
          {(dc) => (
            <section class="picker-dc">
              <header>
                <strong>{dc.dcName}</strong>
                <Show when={dc.dcId}>
                  <button type="button" class="ghost tiny" onClick={() => addAll(selectByDc(props.cands, dc.dcId))}>Whole data center</button>
                </Show>
              </header>
              <For each={dc.sites}>
                {(site) => (
                  <div class="picker-site">
                    <header>
                      <span class="eyebrow">{site.siteName}</span>
                      <Show when={site.siteId}>
                        <button
                          type="button" class="ghost tiny"
                          onClick={() => { setLastSite(site.siteId); addAll(selectBySite(props.cands, site.siteId)); }}
                        >
                          Whole site
                        </button>
                      </Show>
                    </header>
                    <ul>
                      <For each={site.nodes}>
                        {(node) => (
                          <li classList={{ chosen: props.selected.has(node.id) }}>
                            <label>
                              <input type="checkbox" checked={props.selected.has(node.id)} onChange={() => flip(node.id)} />
                              <span class="mono">{node.name}</span>
                              <Show when={node.gpu}><span class="tag">GPU</span></Show>
                            </label>
                            <span class={`badge ${TONES[node.fit]}`} title={node.reason}><i />{WORDS[node.fit]}</span>
                            <span class="muted small">{node.reason}</span>
                          </li>
                        )}
                      </For>
                    </ul>
                  </div>
                )}
              </For>
            </section>
          )}
        </For>
      </Show>
    </div>
  );
}
