// The sandbox Admin Center: one sandbox, full screen, one tab per question.
//
// This file is the frame, not a feature: breadcrumb, a sticky title row with status and presence,
// the action bar (ActionBar.jsx), the tab strip and the body switch. Each tab body lives in its
// own file and is owned by one package. The tab list (ids, labels, who may see them) is data in
// adminTabs.js; this file only maps a tab key to its component.
//
// What every tab receives:
//   props.sandbox   the stored sandbox row (name, pack, image, status, runtime)
//   props.reload    re-read the sandbox after an action
//   props.revision  bumped by `reload`, for a tab that wants to refetch
//
// The active tab lives in the URL (`?tab=`): `navigate(paths.sandbox(id, tab))` writes it and
// `props.initialTab` (from parsePath) reads it, so a reload or a shared link lands on the same tab.

import { Show, For, createSignal, createEffect, createMemo, onCleanup, untrack } from 'solid-js';
import Tabs from '../../components/Tabs.jsx';
import TabBoundary from '../../components/TabBoundary.jsx';
import StatusBadge from '../../components/StatusBadge.jsx';
import ConsoleFrame from '../../components/ConsoleFrame.jsx';
import PresenceDots from '../../components/PresenceDots.jsx';
import Segmented from '../../components/Segmented.jsx';
import { usePolling } from '../../components/Poll.js';
import Breadcrumb from '../shell/Breadcrumb.jsx';
import PageHeader from '../shell/PageHeader.jsx';
import { useInventory } from '../shell/useInventory.js';
import { treePathFor } from '../shell/treeFilter.js';
import { useSession } from '../../state/session.jsx';
import { useUi } from '../../state/ui.jsx';
import { navigate, paths } from '../../routing.js';
import * as api from '../../api/sandboxes.js';
import * as controlApi from '../../api/control.js';
import ActionBar from './ActionBar.jsx';
import SummaryTab from './SummaryTab.jsx';
import { resolveTab, subViewFor, tabsFor } from './adminTabs.js';
import DashboardTab from './DashboardTab.jsx';
import TrafficTab from './TrafficTab.jsx';
import McpTab from './McpTab.jsx';
import AccessTab from './AccessTab.jsx';
import MapTab from './MapTab.jsx';
import SupervisorTab from './SupervisorTab.jsx';
import SessionsTab from './SessionsTab.jsx';
import '../../styles/sandbox-hierarchy.css';
import '../../styles/admin-center.css';
import PoliciesTab from './PoliciesTab.jsx';
import ProcessesTab from './ProcessesTab.jsx';
import ConnectionsTab from './ConnectionsTab.jsx';
import ChatsTab from './ChatsTab.jsx';
import UsageTab from './UsageTab.jsx';
import CommandsTab from './CommandsTab.jsx';
import InventoryTab from './InventoryTab.jsx';
import LogsTab from './LogsTab.jsx';
import MountEditor from './MountEditor.jsx';
import PrivacyTab from './PrivacyTab.jsx';
import AgentsTab from './AgentsTab.jsx';
import PermissionsTab from '../users/PermissionsTab.jsx';

const SHORT_DIGEST = 19;
const CONTROL_POLL_MS = 10000;

// Live: the three "what is it doing right now" tabs behind one switch.
const LIVE_VIEWS = [
  { value: 'connections', label: 'Connections' },
  { value: 'commands', label: 'Commands' },
  { value: 'chats', label: 'Chats' },
];
const LIVE_COMPONENTS = { connections: ConnectionsTab, commands: CommandsTab, chats: ChatsTab };

// Console: the shared frame (components/ConsoleFrame.jsx) owns Shell, Desktop and SSH.


// A tab that groups several views: a switch on top, the chosen view below.
function Grouped(props) {
  return (
    <div class="ac-group">
      <Segmented options={props.views} value={props.view} onChange={props.onView} />
      <For each={props.views}>
        {(entry) => (
          <Show when={entry.value === props.view}>
            {(() => {
              const View = props.components[entry.value];
              return <View sandbox={props.sandbox} revision={props.revision} reload={props.reload} followed={props.followed} />;
            })()}
          </Show>
        )}
      </For>
    </div>
  );
}

// Tab key (adminTabs.js `component`) to a render function receiving the shared tab props.
const BODIES = {
  summary: (p) => <SummaryTab sandbox={p.sandbox} control={p.control} revision={p.revision} onTab={p.goTab} />,
  live: (p) => <Grouped views={LIVE_VIEWS} components={LIVE_COMPONENTS} view={p.liveView()} onView={p.setLiveView} {...p.base} />,
  console: (p) => <ConsoleFrame sandbox={p.base.sandbox} mode={p.consoleView() || undefined} onMode={p.setConsoleView} />,
  traffic: (p) => <TrafficTab {...p.base} />,
  policies: (p) => (
    <div>
      <div class="ac-tab-actions">
        <button class="secondary tiny" onClick={() => navigate(`/policies?wizard=1&sandbox=${p.sandbox.id}`)}>Open policy wizard</button>
      </div>
      <PoliciesTab {...p.base} />
    </div>
  ),
  mounts: (p) => <MountEditor sandboxId={p.sandbox.id} />,
  access: (p) => <AccessTab {...p.base} />,
  mcp: (p) => <McpTab {...p.base} />,
  agents: (p) => <AgentsTab {...p.base} />,
  processes: (p) => <ProcessesTab {...p.base} />,
  inventory: (p) => <InventoryTab {...p.base} />,
  logs: (p) => <LogsTab {...p.base} />,
  privacy: (p) => <PrivacyTab {...p.base} />,
  usage: (p) => <UsageTab {...p.base} />,
  resources: (p) => <DashboardTab {...p.base} />,
  sessions: (p) => <SessionsTab {...p.base} />,
  supervisor: (p) => <SupervisorTab {...p.base} />,
  map: (p) => <MapTab {...p.base} />,
  permissions: (p) => <PermissionsTab scopeType="sandbox" scopeId={p.base.sandbox.id} />,
};

export default function AdminCenter(props) {
  const session = useSession();
  const ui = useUi();
  const inventory = useInventory();
  const role = () => session?.identity()?.role;
  const tab = createMemo(() => resolveTab(props.initialTab, role()));
  const visibleTabs = createMemo(() => tabsFor(role()));
  const [liveView, setLiveView] = createSignal('connections');
  // The console view: set by an old link (`?tab=terminal` / `?tab=desktop`) or by the split
  // button; null lets ConsoleFrame use the last choice for this sandbox.
  const [consoleView, setConsoleView] = createSignal(null);
  const [sandbox, setSandbox] = createSignal(null);
  const [control, setControl] = createSignal(null);
  const [error, setError] = createSignal(null);
  // Bumped by `reload`, so a tab can depend on it without owning the fetch.
  const [revision, setRevision] = createSignal(0);

  async function load() {
    const id = props.sandboxId;
    if (!id) {
      setSandbox(null);
      return;
    }
    try {
      setSandbox(await api.get(id));
      setError(null);
    } catch (failure) {
      setError(failure.message || 'The sandbox could not be read.');
    }
  }

  function reload() {
    setRevision(revision() + 1);
    load();
    refreshControl();
  }

  async function refreshControl() {
    const id = props.sandboxId;
    if (!id) return;
    try {
      const answer = await controlApi.summary(id);
      if (id === props.sandboxId) setControl(answer);
    } catch {
      // The summary is a convenience: the header and tabs work without it.
    }
  }

  usePolling(() => controlApi.summary(props.sandboxId), {
    everyMs: CONTROL_POLL_MS,
    onData: (answer) => setControl(answer),
    onError: () => {},
  });

  createEffect(() => {
    // Re-read whenever a different sandbox is opened.
    props.sandboxId;
    setControl(null);
    load();
  });

  // An old link (`?tab=terminal`) opens the right view of the grouped tab.
  createEffect(() => {
    const view = subViewFor(props.initialTab);
    if (view === 'shell' || view === 'desktop') setConsoleView(view);
    else if (view) setLiveView(view);
  });

  // Close on Escape: a full-screen view with no browser chrome needs a way out, but not while a
  // dialog or a menu is using the key.
  function onKey(event) {
    if (event.key !== 'Escape') return;
    if (document.querySelector('.nb-dialog, .ac-menu')) return;
    props.onClose();
  }
  window.addEventListener('keydown', onKey);
  onCleanup(() => window.removeEventListener('keydown', onKey));

  function goTab(next) {
    navigate(paths.sandbox(props.sandboxId, next));
  }

  // The Console split button: Shell and Desktop switch the console tab, SSH opens Access.
  function openConsole(mode) {
    if (mode === 'ssh') {
      goTab('access');
      return;
    }
    ui?.console.setMode(props.sandboxId, mode);
    setConsoleView(mode);
    goTab('console');
  }

  const digest = () => {
    const value = sandbox()?.image_digest;
    if (!value) return 'not pinned';
    return value.length > SHORT_DIGEST ? `${value.slice(0, SHORT_DIGEST)}…` : value;
  };
  const address = () => {
    const networks = sandbox()?.runtime?.network || [];
    const first = networks.find((entry) => entry.ip) || networks[0];
    return first?.ip || '—';
  };
  const people = () =>
    (Array.isArray(control()?.presence) ? control().presence : []).map((entry) => ({
      name: entry.user_name || 'SSH',
      kind: entry.kind,
      view_only: entry.view_only,
    }));
  const crumbs = () => {
    const trail = treePathFor(inventory.root(), 'admin', props.sandboxId);
    if (trail.length > 0) return trail;
    const items = [{ label: 'Sandboxes', path: paths.menu('sandboxes') }];
    const node = control()?.node?.name || sandbox()?.node_name;
    if (node) items.push({ label: node });
    items.push({ label: sandbox()?.name || props.sandboxId });
    return items;
  };

  // Getters, so a body built once keeps following the signals (a rebuilt body would remount the tab).
  const ctx = {
    get sandbox() { return sandbox(); },
    get control() { return control(); },
    get revision() { return revision(); },
    goTab,
    liveView,
    setLiveView,
    consoleView,
    setConsoleView,
    base: {
      get sandbox() { return sandbox(); },
      get revision() { return revision(); },
      reload,
      followed: props.onFollow,
    },
  };

  return (
    <div class="admin-center">
      <Breadcrumb items={crumbs()} />
      <PageHeader
        title={sandbox()?.name || props.sandboxId}
        status={sandbox() ? <StatusBadge status={sandbox().status} /> : null}
        subtitle={
          sandbox() ? (
            <>
              {sandbox().pack_key || 'pack'} · {sandbox().image || 'image'} · <span class="mono">{digest()}</span> ·{' '}
              {sandbox().node_name || 'local'} · <span class="mono">{address()}</span>
            </>
          ) : null
        }
        actions={
          <>
            <PresenceDots people={people()} />
            <Show when={sandbox()}>
              <ActionBar
                sandbox={sandbox()}
                control={control()}
                role={role()}
                onChanged={reload}
                onControl={setControl}
                onTab={goTab}
                onConsole={openConsole}
              />
            </Show>
          </>
        }
      />

      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>

      <Tabs tabs={visibleTabs()} active={tab()} onChange={goTab} />

      <section class="ac-body" role="tabpanel">
        <Show
          when={sandbox()}
          fallback={<p class="muted">Loading {props.sandboxId} …</p>}
        >
          <For each={visibleTabs()}>
            {(entry) => (
              <Show when={entry.id === tab()}>
                <TabBoundary>
                  {untrack(() => BODIES[entry.component](ctx))}
                </TabBoundary>
              </Show>
            )}
          </For>
        </Show>
      </section>
    </div>
  );
}
