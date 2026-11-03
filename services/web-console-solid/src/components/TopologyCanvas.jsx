// One layered topology drawing for Home, Data center, Node and Cluster pages: zoom, pan,
// minimap, a side panel for the selected card and an optional traffic-window switch.
//
// Props in, callbacks out (contract C15). The host maps its data to nodes and edges and
// handles `onAction(id, 'open' | 'policies' | 'logs' | 'allow:15' | 'allow:30' | 'allow:60')`.
// Extra optional props: `trafficWindows` (e.g. ['', '5m', '15m', '1h']), `trafficWindow`,
// `onTrafficWindow(value)`, `panel` (false hides the built-in side panel), `busy`.

import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from 'solid-js';
import Segmented from './Segmented.jsx';
import Minimap from './Minimap.jsx';
import TopologyPanel from './TopologyPanel.jsx';
import { Icon } from './icons.jsx';
import { DEFAULTS, edgePath, fit, layout, thickness, toneOf, zoomAt } from './topologyLayout.js';
import '../styles/topology.css';

// The icon per card kind (`components/icons.jsx`); unknown kinds get Settings.
const KIND_ICONS = {
  organization: 'Home',
  data_center: 'DataCenter',
  site: 'Site',
  node: 'Node',
  gateway: 'Network',
  supervisor: 'Policy',
  sandbox: 'Sandbox',
  egress: 'Traffic',
  destination: 'Network',
  model: 'Model',
  cluster: 'Cluster',
  user: 'User',
};

// Cut a label so it fits a card.
function cut(text, max) {
  const value = String(text ?? '');
  return value.length > max ? `${value.slice(0, max - 1)}…` : value;
}

export default function TopologyCanvas(props) {
  const card = DEFAULTS.card;
  const height = () => props.height || 520;
  const [box, setBox] = createSignal({ w: 900, h: height() });
  const [vb, setVb] = createSignal({ x: 0, y: 0, w: 900, h: height() });
  // Once the user zooms or pans, new data no longer re-fits the view.
  const [touched, setTouched] = createSignal(false);
  let wrap;
  let drag = null;

  const placed = createMemo(() => layout(props.nodes || [], props.edges || [], props.layers || []));
  const byId = createMemo(() => new Map((props.nodes || []).map((node) => [node.id, node])));
  const selected = () => byId().get(props.selectedId) || null;
  const cards = createMemo(() => (props.nodes || [])
    .filter((node) => placed().positions.has(node.id))
    .map((node) => ({ node, ...placed().positions.get(node.id), tone: toneOf(node.status) })));
  const edges = createMemo(() => (props.edges || [])
    .map((edge) => ({ edge, a: placed().positions.get(edge.from), b: placed().positions.get(edge.to) }))
    .filter((item) => item.a && item.b));
  // Edges around the selected card stay bright; the rest dims.
  const related = (edge) => !props.selectedId || edge.from === props.selectedId || edge.to === props.selectedId;
  const titles = createMemo(() => (props.layers || []).map((name, index) => ({ name, x: placed().pad + index * DEFAULTS.colGap })));

  function fitView() {
    setVb(fit(placed().width, placed().height, box()));
  }

  onMount(() => {
    const measure = () => {
      if (!wrap) return;
      setBox({ w: Math.max(wrap.clientWidth, 200), h: height() });
      if (!touched()) fitView();
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(wrap);
    onCleanup(() => observer.disconnect());
  });

  createEffect(() => {
    placed();
    if (!touched()) fitView();
  });

  function zoom(factor, px = 0.5, py = 0.5) {
    setTouched(true);
    setVb((view) => zoomAt(view, factor, px, py));
  }

  function onWheel(event) {
    event.preventDefault();
    const rect = event.currentTarget.getBoundingClientRect();
    zoom(event.deltaY > 0 ? 1.1 : 0.9, (event.clientX - rect.left) / rect.width, (event.clientY - rect.top) / rect.height);
  }

  function onDown(event) {
    if (!event.target.classList.contains('topo-bg')) return;
    drag = { x: event.clientX, y: event.clientY, vb: vb(), width: event.currentTarget.getBoundingClientRect().width };
    props.onSelect?.(null);
  }

  function onMove(event) {
    if (!drag) return;
    setTouched(true);
    const scale = drag.vb.w / drag.width;
    setVb({ ...drag.vb, x: drag.vb.x - (event.clientX - drag.x) * scale, y: drag.vb.y - (event.clientY - drag.y) * scale });
  }

  function onUp() {
    drag = null;
  }

  function onKey(event) {
    if (event.target.closest?.('input, select, textarea')) return;
    if (event.key === '+' || event.key === '=') zoom(0.8);
    else if (event.key === '-' || event.key === '_') zoom(1.25);
    else if (event.key === '0') {
      setTouched(false);
      fitView();
    }
  }

  function cardKey(event, id) {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      props.onSelect?.(id);
    }
  }

  function moveTo(x, y) {
    setTouched(true);
    setVb((view) => ({ ...view, x: x - view.w / 2, y: y - view.h / 2 }));
  }

  const trafficOptions = () => (props.trafficWindows || []).map((value) => ({ value, label: value === '' ? 'Off' : value }));

  return (
    <div class="topo" ref={wrap} onKeyDown={onKey}>
      <div class="topo-stage">
        <Show
          when={(props.nodes || []).length > 0}
          fallback={<p class="muted topo-empty">Nothing to draw yet.</p>}
        >
          <svg
            class="topo-svg"
            width="100%"
            height={height()}
            viewBox={`${vb().x} ${vb().y} ${vb().w} ${vb().h}`}
            role="group"
            aria-label={props.label || 'Topology'}
            onWheel={onWheel}
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerLeave={onUp}
          >
            <rect class="topo-bg" x={vb().x - 4000} y={vb().y - 4000} width={vb().w + 8000} height={vb().h + 8000} fill="transparent" />
            <For each={titles()}>
              {(title) => <text class="topo-layer" x={title.x} y={placed().pad + 10}>{title.name}</text>}
            </For>
            <g class="edges">
              <For each={edges()}>
                {(item) => (
                  <g class={`edge${item.edge.refused ? ' bad' : ''}${item.edge.requests ? ' traffic' : ''}${related(item.edge) ? '' : ' dim'}`}>
                    <path d={edgePath(item.a, item.b, card)} stroke-width={thickness(item.edge.requests)} />
                    <Show when={item.edge.refused}>
                      <path
                        class="edge-hit"
                        d={edgePath(item.a, item.b, card)}
                        onClick={() => props.onSelect?.(item.edge.to)}
                      />
                    </Show>
                  </g>
                )}
              </For>
            </g>
            <g class="nodes">
              <For each={cards()}>
                {(item) => (
                  <g
                    class={`topo-card ${item.tone} kind-${item.node.kind}${props.selectedId === item.node.id ? ' selected' : ''}`}
                    transform={`translate(${item.x}, ${item.y})`}
                    role="button"
                    tabindex="0"
                    aria-label={`${item.node.kind} ${item.node.label}, ${item.node.statusLabel || item.node.status || 'unknown'}`}
                    aria-pressed={props.selectedId === item.node.id}
                    onClick={() => props.onSelect?.(item.node.id)}
                    onDblClick={() => props.onAction?.(item.node.id, 'open')}
                    onKeyDown={(event) => cardKey(event, item.node.id)}
                  >
                    <rect width={card.w} height={card.h} />
                    <g transform="translate(10, 10)">
                      <Icon name={KIND_ICONS[item.node.kind] || 'Settings'} />
                    </g>
                    <text class="card-label" x="32" y="21">{cut(item.node.label, 18)}</text>
                    <text class="card-state" x="32" y="39">
                      {item.node.statusLabel || item.node.status || item.node.kind}
                      {item.node.meta?.running !== undefined ? ` · ${item.node.meta.running} running` : ''}
                      {item.node.meta?.alerts ? ` · ${item.node.meta.alerts} alerts` : ''}
                    </text>
                    <circle class="card-dot" cx={card.w - 12} cy="12" r="4" />
                  </g>
                )}
              </For>
            </g>
          </svg>

          <div class="topo-toolbar">
            <Show when={trafficOptions().length > 0}>
              <span class="muted small">Traffic</span>
              <Segmented options={trafficOptions()} value={props.trafficWindow ?? ''} onChange={(value) => props.onTrafficWindow?.(value)} />
            </Show>
            <button class="secondary tiny" aria-label="Zoom in" onClick={() => zoom(0.8)}>+</button>
            <button class="secondary tiny" aria-label="Zoom out" onClick={() => zoom(1.25)}>−</button>
            <button class="secondary tiny" aria-label="Fit the drawing to the view" onClick={() => { setTouched(false); fitView(); }}>Fit</button>
          </div>

          <div class="topo-legend small" aria-label="Legend">
            <span><i class="dot ok" /> ok</span>
            <span><i class="dot warn" /> attention</span>
            <span><i class="dot bad" /> failing</span>
            <span><i class="dot muted" /> unknown</span>
            <span class="legend-refused">red line: refused</span>
          </div>

          <div class="topo-mini">
            <Minimap
              width={placed().width}
              height={placed().height}
              card={card}
              cards={cards()}
              view={vb()}
              onMove={moveTo}
            />
          </div>
        </Show>

        <Show when={selected() && props.panel !== false}>
          <TopologyPanel
            node={selected()}
            nodes={props.nodes}
            edges={props.edges}
            busy={props.busy}
            onSelect={(id) => props.onSelect?.(id)}
            onClose={() => props.onSelect?.(null)}
            onAction={(id, action) => props.onAction?.(id, action)}
          />
        </Show>
      </div>
    </div>
  );
}
