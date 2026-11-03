// The display arrangement of a desktop sandbox, like the arrangement in an OS
// display dialog (U10c): one rectangle per monitor, dragged into place; the mode
// as two buttons; the selected monitor's resolution, rotation and primary flag;
// and the live counters that name a limit the moment it is hit.
//
// Props in, callbacks out. The component owns the drawing and the drag, and it
// takes the layout maths from `features/sandboxes/displayLayout.js` — a pure
// module with no state and no fetching, the same exception `TerminalTab` makes
// for the shared tab strip: a second copy of the rules is what the exception is
// there to prevent. Nothing here decides what is allowed; the layout it hands
// back is checked by the pure validator and by the sandbox.
//
// A drag snaps while it moves: the rectangle jumps to the nearest free slot
// beside a neighbour, so what is dropped is exactly what was shown. The write
// itself is the caller's: this component only says "monitor m2 moved to 1920,0".
import { For, Show, createEffect, createMemo, createSignal, onCleanup } from 'solid-js';
import Segmented from './Segmented.jsx';
import {
  MAX_AREA,
  MAX_MONITORS,
  MODE_SEPARATE,
  MODE_STRETCHED,
  arrangeGeometry,
  monitorSize,
  snap,
  totals,
} from '../features/sandboxes/displayLayout.js';
import '../styles/display.css';

const MODES = [
  { value: MODE_SEPARATE, label: 'Separate screens', title: 'Each monitor is its own screen: panels, maximize and fullscreen stay on it' },
  { value: MODE_STRETCHED, label: 'One stretched screen', title: 'One screen over all monitors: maximize and fullscreen fill them all' },
];

export default function DisplayArrange(props) {
  let host;
  const [width, setWidth] = createSignal(228);
  const [drag, setDrag] = createSignal(null);
  const [chosen, setChosen] = createSignal(null);
  const [fields, setFields] = createSignal({ width: 0, height: 0 });

  // The drawing is as wide as its column; measuring it keeps one pixel in the SVG
  // one CSS pixel, so a drag converts to layout pixels with a single division.
  if (typeof ResizeObserver !== 'undefined') {
    const observer = new ResizeObserver(() => {
      if (host) setWidth(Math.max(80, host.clientWidth));
    });
    onCleanup(() => observer.disconnect());
    createEffect(() => {
      if (host) observer.observe(host);
    });
  }

  const monitors = () => props.layout?.monitors || [];
  const geometry = createMemo(() => arrangeGeometry(monitors(), width()));
  const counters = createMemo(() => totals(props.layout));

  // The selected monitor is the caller's when it names one, else the last picked,
  // else the primary, else the first. It is a view of the layout, not state of its own.
  const selectedId = () => {
    const wanted = props.selected || chosen();
    const found = monitors().find((monitor) => monitor.id === wanted);
    if (found) return found.id;
    const primary = monitors().find((monitor) => monitor.primary) || monitors()[0];
    return primary?.id || null;
  };
  const selected = () => monitors().find((monitor) => monitor.id === selectedId()) || null;

  // The number fields follow the selection, but not while a person is typing in
  // them: only an outside change (a preset, a drag, an agent) rewrites the boxes.
  createEffect(() => {
    const monitor = selected();
    if (monitor) setFields({ width: monitor.width, height: monitor.height });
  });

  function pick(id) {
    setChosen(id);
    props.onSelect?.(id);
  }

  // A drag is a delta from where it started: the layout moves by the same
  // translation as the pointer, divided by the drawing's scale.
  function apply(clientX, clientY) {
    const moving = drag();
    if (!moving) return;
    const scale = geometry().scale || 1;
    const dx = (clientX - moving.start.x) / scale;
    const dy = (clientY - moving.start.y) / scale;
    setDrag({
      ...moving,
      at: snap(moving.monitors, moving.id, moving.origin.x + dx, moving.origin.y + dy),
    });
  }

  function onMove(event) {
    apply(event.clientX, event.clientY);
  }

  function onUp() {
    const moving = drag();
    stopDrag();
    if (!moving) return;
    const at = moving.at;
    if (at && (at.x !== moving.origin.x || at.y !== moving.origin.y)) {
      props.onMove?.(moving.id, at.x, at.y);
    }
  }

  function stopDrag() {
    setDrag(null);
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onUp);
  }

  onCleanup(stopDrag);

  function begin(event, monitor) {
    if (event.button !== undefined && event.button !== 0) return;
    event.preventDefault();
    pick(monitor.id);
    setDrag({
      id: monitor.id,
      start: { x: event.clientX, y: event.clientY },
      origin: { x: monitor.x, y: monitor.y },
      monitors: monitors().map((entry) => ({ ...entry })),
      at: { x: monitor.x, y: monitor.y },
    });
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    window.addEventListener('pointercancel', onUp);
  }

  // Where one monitor's rectangle is drawn: the dragged one at its snapped place.
  function place(monitor) {
    const moving = drag();
    const at = moving?.id === monitor.id && moving.at ? moving.at : monitor;
    const box = geometry().box;
    const scale = geometry().scale;
    return {
      x: Math.round(geometry().pad + (at.x - box.left) * scale),
      y: Math.round(geometry().pad + (at.y - box.top) * scale),
      w: Math.max(8, Math.round(monitor.width * scale)),
      h: Math.max(8, Math.round(monitor.height * scale)),
    };
  }

  const sizeOf = (monitor) => {
    const known = (props.presets || []).find((entry) => entry.width === monitor.width && entry.height === monitor.height);
    return known ? known.label : 'custom';
  };

  return (
    <section class="display-arrange" ref={host} aria-label="Display arrangement">
      <svg
        class="display-svg"
        viewBox={`0 0 ${geometry().width} ${geometry().height}`}
        width="100%"
        height={geometry().height}
        role="group"
        aria-label={`${monitors().length} monitors, ${props.layout?.mode === MODE_STRETCHED ? 'one stretched screen' : 'separate screens'}`}
      >
        <For each={monitors()}>
          {(monitor) => {
            const at = () => place(monitor);
            return (
              <g
                class="display-monitor"
                classList={{
                  active: selectedId() === monitor.id,
                  dragging: drag()?.id === monitor.id,
                }}
                transform={`translate(${at().x}, ${at().y})`}
                onPointerDown={(event) => begin(event, monitor)}
                role="button"
                tabindex="0"
                aria-label={`${monitor.name}, ${monitorSize(monitor)} at ${monitor.x},${monitor.y}`}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    pick(monitor.id);
                  }
                }}
              >
                <rect width={at().w} height={at().h} />
                <text x="4" y="13">{monitor.name}</text>
                <Show when={at().h >= 34}>
                  <text x="4" y="26" class="display-size">{monitorSize(monitor)}</text>
                </Show>
                <Show when={monitor.primary}>
                  <circle cx={at().w - 7} cy="7" r="3" class="display-primary" />
                </Show>
              </g>
            );
          }}
        </For>
      </svg>

      <Segmented options={MODES} value={props.layout?.mode} onChange={(mode) => props.onMode?.(mode)} />

      <div class="display-row">
        <label class="display-field">
          <span>Monitor</span>
          <select value={selectedId() || ''} onChange={(event) => pick(event.currentTarget.value)}>
            <For each={monitors()}>
              {(monitor) => <option value={monitor.id}>{monitor.name}</option>}
            </For>
          </select>
        </label>
        <Show when={selected()}>
          {(monitor) => (
            <label class="display-field">
              <span>Resolution</span>
              <select
                value={sizeOf(monitor())}
                onChange={(event) => {
                  const found = (props.presets || []).find((entry) => entry.label === event.currentTarget.value);
                  if (found) props.onResize?.(monitor().id, found.width, found.height);
                }}
              >
                <For each={props.presets || []}>
                  {(entry) => <option value={entry.label}>{entry.label}</option>}
                </For>
                <option value="custom">{`Custom ${monitorSize(monitor())}`}</option>
              </select>
            </label>
          )}
        </Show>
      </div>

      <Show when={selected()}>
        {(monitor) => (
          <>
            <div class="display-row">
              <label class="display-field">
                <span>W</span>
                <input
                  type="number"
                  min="320"
                  max="7680"
                  class="mono"
                  value={fields().width}
                  onChange={(event) => props.onResize?.(monitor().id, Number(event.currentTarget.value), monitor().height)}
                />
              </label>
              <label class="display-field">
                <span>H</span>
                <input
                  type="number"
                  min="320"
                  max="7680"
                  class="mono"
                  value={fields().height}
                  onChange={(event) => props.onResize?.(monitor().id, monitor().width, Number(event.currentTarget.value))}
                />
              </label>
              <button
                type="button"
                class="tiny secondary"
                title="Turn this monitor by a quarter"
                onClick={() => props.onRotate?.(monitor().id)}
              >
                ⟲ Rotate
              </button>
            </div>
            <div class="display-row">
              <span class="mono small muted">
                {monitor().name} · {monitorSize(monitor())} at {monitor().x},{monitor().y}
              </span>
              <span class="display-spacer" />
              <button
                type="button"
                class="tiny secondary"
                disabled={monitor().primary === true}
                onClick={() => props.onPrimary?.(monitor().id)}
              >
                Make primary
              </button>
            </div>
          </>
        )}
      </Show>

      <p class="display-counters" classList={{ over: counters().overArea || counters().overBox || counters().overCount }}>
        <span class="mono">{counters().width}×{counters().height}</span>
        {' · '}
        <span class="mono">{counters().count}/{MAX_MONITORS} monitors</span>
        {' · '}
        <span class="mono">{counters().megapixels.toFixed(1)} MP of {Math.round(MAX_AREA / 1_000_000)} MP</span>
      </p>

      <Show when={props.status?.text}>
        <p class="display-status" data-kind={props.status.kind}>{props.status.text}</p>
      </Show>
      <Show when={(props.problems || []).length > 1}>
        <ul class="display-problems">
          <For each={(props.problems || []).slice(1, 4)}>
            {(problem) => <li>{problem}</li>}
          </For>
        </ul>
      </Show>
      <Show when={(props.layout?.monitors || []).length === 0}>
        <p class="muted small">The sandbox reports no monitor. A running desktop pack answers this.</p>
      </Show>
    </section>
  );
}
