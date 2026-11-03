// The dock at the bottom: Tasks, Live events and Alerts. Collapsed to one 28 px bar by default;
// the top edge resizes it (120-480 px). Under 720 px it is a bottom sheet behind a floating button.
import { Show, createSignal } from 'solid-js';
import { useUi } from '../../state/ui.jsx';
import { paths } from '../../routing.js';
import TasksTab from '../dock/TasksTab.jsx';
import LiveEventsTab from '../dock/LiveEventsTab.jsx';
import AlertsTab from '../dock/AlertsTab.jsx';

const TABS = [
  { id: 'tasks', label: 'Tasks' },
  { id: 'events', label: 'Live events' },
  { id: 'alerts', label: 'Alerts' },
];
const MIN_HEIGHT = 120;
const MAX_HEIGHT = 480;

// Open a pop-out window for the logs or traffic view; tasks have none.
function popOut(tab) {
  const path = tab === 'events' ? paths.popLogs('all') : tab === 'alerts' ? paths.popTraffic() : null;
  if (path) window.open(path, `nestbox-${tab}`, 'width=1100,height=700');
}

export default function Dock() {
  const ui = useUi();
  const [sheet, setSheet] = createSignal(false);
  const clamp = (value) => Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, Math.round(value)));

  let drag = null;
  function onDown(event) {
    drag = { y: event.clientY, height: ui.dock.height() };
    event.currentTarget.setPointerCapture?.(event.pointerId);
  }
  function onMove(event) {
    if (drag) ui.dock.setHeight(clamp(drag.height - (event.clientY - drag.y)));
  }
  function onUp() {
    drag = null;
  }
  function onHandleKey(event) {
    if (event.key === 'ArrowUp') ui.dock.setHeight(clamp(ui.dock.height() + 16));
    else if (event.key === 'ArrowDown') ui.dock.setHeight(clamp(ui.dock.height() - 16));
    else return;
    event.preventDefault();
  }

  const canPop = () => ui.dock.tab() === 'events' || ui.dock.tab() === 'alerts';

  return (
    <>
      <button
        class="secondary cc-dock-fab"
        aria-label="Open the dock"
        aria-expanded={sheet()}
        onClick={() => { setSheet(!sheet()); ui.dock.setOpen(true); }}
      >
        Dock
        <Show when={ui.alerts.count() > 0}><span class="cc-bell-count hot mono">{ui.alerts.count()}</span></Show>
      </button>
      <section class="cc-dock" classList={{ open: ui.dock.open(), sheet: sheet() }} data-tour="dock" aria-label="Dock">
        <Show when={ui.dock.open()}>
          <div
            class="cc-dock-resizer"
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize the dock"
            aria-valuemin={MIN_HEIGHT}
            aria-valuemax={MAX_HEIGHT}
            aria-valuenow={ui.dock.height()}
            tabindex="0"
            onPointerDown={onDown}
            onPointerMove={onMove}
            onPointerUp={onUp}
            onPointerCancel={onUp}
            onKeyDown={onHandleKey}
          />
        </Show>
        <div class="cc-dock-bar" role="tablist" aria-label="Dock tabs">
          {TABS.map((tab) => (
            <button
              role="tab"
              aria-selected={ui.dock.tab() === tab.id}
              onClick={() => { ui.dock.setTab(tab.id); ui.dock.setOpen(true); }}
            >
              {tab.label}
              <Show when={tab.id === 'alerts' && ui.alerts.count() > 0}>
                <span class="cc-bell-count hot mono">{ui.alerts.count()}</span>
              </Show>
            </button>
          ))}
          <Show when={canPop()}>
            <button class="ghost cc-dock-pop" aria-label="Pop out this tab into its own window" onClick={() => popOut(ui.dock.tab())}>
              Pop out
            </button>
          </Show>
          <button
            class="ghost cc-dock-toggle"
            aria-label={ui.dock.open() ? 'Collapse the dock' : 'Expand the dock'}
            aria-expanded={ui.dock.open()}
            onClick={() => { const next = !ui.dock.open(); ui.dock.setOpen(next); if (!next) setSheet(false); }}
          >
            {ui.dock.open() ? 'Collapse ▾' : 'Expand ▴'}
          </button>
        </div>
        <Show when={ui.dock.open()}>
          <div class="cc-dock-body" role="tabpanel" style={{ height: `${ui.dock.height()}px` }}>
            <Show when={ui.dock.tab() === 'tasks'}><TasksTab /></Show>
            <Show when={ui.dock.tab() === 'events'}><LiveEventsTab /></Show>
            <Show when={ui.dock.tab() === 'alerts'}><AlertsTab /></Show>
          </div>
        </Show>
      </section>
    </>
  );
}
