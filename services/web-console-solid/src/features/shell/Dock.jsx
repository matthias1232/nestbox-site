// The dock at the bottom: Tasks, Live events and Alerts. Collapsed to one bar by default.
// Skeleton from Wave 0: it toggles open and shows the placeholder tabs; F13 fills them.
import { Show } from 'solid-js';
import { useUi } from '../../state/ui.jsx';
import TasksTab from '../dock/TasksTab.jsx';
import LiveEventsTab from '../dock/LiveEventsTab.jsx';
import AlertsTab from '../dock/AlertsTab.jsx';

const TABS = [
  { id: 'tasks', label: 'Tasks' },
  { id: 'events', label: 'Live events' },
  { id: 'alerts', label: 'Alerts' },
];

export default function Dock() {
  const ui = useUi();
  return (
    <section class="cc-dock" classList={{ open: ui.dock.open() }} data-tour="dock" aria-label="Dock">
      <div class="cc-dock-bar" role="tablist">
        {TABS.map((tab) => (
          <button
            role="tab"
            aria-selected={ui.dock.tab() === tab.id}
            onClick={() => { ui.dock.setTab(tab.id); ui.dock.setOpen(true); }}
          >
            {tab.label}
          </button>
        ))}
        <button class="ghost cc-dock-toggle" aria-expanded={ui.dock.open()} onClick={() => ui.dock.setOpen(!ui.dock.open())}>
          {ui.dock.open() ? 'Collapse' : 'Expand'}
        </button>
      </div>
      <Show when={ui.dock.open()}>
        <div class="cc-dock-body" style={{ height: `${ui.dock.height()}px` }}>
          <Show when={ui.dock.tab() === 'tasks'}><TasksTab /></Show>
          <Show when={ui.dock.tab() === 'events'}><LiveEventsTab /></Show>
          <Show when={ui.dock.tab() === 'alerts'}><AlertsTab /></Show>
        </div>
      </Show>
    </section>
  );
}
