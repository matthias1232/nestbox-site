// The sandbox inventory.
//
// Density is deliberate: an operator scanning for a problem wants many rows and
// few pixels. Detail lives in the drawer.

import HostLoad from '../../components/HostLoad.jsx';
import { For, Show, onCleanup, onMount } from 'solid-js';
import StatusBadge from '../../components/StatusBadge.jsx';
import Empty from '../../components/Empty.jsx';
import Drawer from '../../components/Drawer.jsx';
import { useSandboxes } from './useSandboxes.js';
import { useUi } from '../../state/ui.jsx';
import SandboxDetail from './SandboxDetail.jsx';
import CreateSandboxForm from './CreateSandboxForm.jsx';
import ConnectPanel from './ConnectPanel.jsx';
import LoginBadge from '../../components/LoginBadge.jsx';
import Countdown from '../../components/Countdown.jsx';
import PresenceDots from '../../components/PresenceDots.jsx';
import * as deck from '../../api/deck.js';
import * as control from '../../api/control.js';
import { navigate } from '../../routing.js';
import { createMemo, createSignal } from 'solid-js';
import { groupStacks, possibleLeads, worstStatus } from './stacks.js';
import { indexById, modeLabel } from './overview.js';
import '../../styles/stacks.css';
import '../../styles/sandbox-hierarchy.css';

export default function SandboxListPage(props) {
  const state = useSandboxes();
  // The bulk selection is shared with the tree and the bulk bar (BulkHost).
  const ui = useUi();
  const isSelected = (id) => ui.selection.kind() === 'sandbox' && ui.selection.ids().has(id);
  const allSelected = () => state.items().length > 0 && state.items().every((item) => isSelected(item.id));
  const [openId, setOpenId] = createSignal(null);
  const [creating, setCreating] = createSignal(false);
  // The sandbox whose connection details are open; `null` closes the drawer.
  const [connecting, setConnecting] = createSignal(null);
  const [created, setCreated] = createSignal(null);

  // How each running sandbox's agent is signed in. Read apart from the list: it asks each
  // sandbox, so it is slower and refreshed less often, and a failure leaves the list intact.
  const [logins, setLogins] = createSignal({});
  async function loadLogins() {
    try {
      const data = await deck.loginSessions();
      setLogins(Object.fromEntries((data.items || []).map((item) => [item.sandbox_id, item])));
    } catch {
      /* the column shows a dash */
    }
  }

  // The cheap per-sandbox control summary: network mode, "all allowed", who is connected.
  // Read every 15 s from the same place as the logins, and a failure leaves the old rows.
  const [summaries, setSummaries] = createSignal({});
  async function loadSummaries() {
    try {
      const list = await control.overview();
      setSummaries(indexById(list));
    } catch {
      /* the columns show a dash */
    }
  }
  onMount(() => {
    state.load({ cursor: null });
    loadLogins();
    loadSummaries();
    const loginTimer = setInterval(loadLogins, 30000);
    const summaryTimer = setInterval(loadSummaries, 15000);
    onCleanup(() => {
      clearInterval(loginTimer);
      clearInterval(summaryTimer);
    });
  });

  // Which stacks are folded shut. Open is the default: a stack that hides its
  // members by default would hide a failing one.
  const [folded, setFolded] = createSignal(new Set());
  const rows = createMemo(() => groupStacks(state.items()));

  function toggleFold(id) {
    const next = new Set(folded());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setFolded(next);
  }

  const leadName = (id) => state.items().find((item) => item.id === id)?.name;

  function confirmDestroy(sandbox) {
    const members = state.items().filter((item) => item.parent_id === sandbox.id).length;
    const note = members > 0
      ? ` Its ${members} member${members === 1 ? '' : 's'} stay${members === 1 ? 's' : ''} as sandboxes of their own.`
      : '';
    if (window.confirm(`Delete sandbox "${sandbox.name}"? Its container and its state are removed.${note}`)) {
      state.destroy(sandbox.id);
    }
  }

  async function createdSandbox(sandbox) {
    setCreating(false);
    setCreated(
      `Sandbox “${sandbox.name}” created on ${sandbox.node_name || 'the local node'}.`,
    );
    await state.load({ cursor: null });
  }


  // One table row. `row` says what the sandbox is in the list: a lead with
  // members, a member under its lead, or a sandbox on its own.
  const rowClass = (sandbox, row) =>
    [
      openId() === sandbox.id ? 'selected' : '',
      row.role === 'lead' ? 'stack-lead' : '',
      row.role === 'member' ? 'stack-member' : '',
    ].join(' ').trim();

  const sandboxRow = (sandbox, row) => (
                  <tr class={rowClass(sandbox, row)}>
                    <td class="narrow stack-gutter">
                      <Show when={row.role === 'lead'}>
                        <button
                          class="fold"
                          aria-label={folded().has(sandbox.id) ? 'Show members' : 'Hide members'}
                          aria-expanded={!folded().has(sandbox.id)}
                          onClick={() => toggleFold(sandbox.id)}
                        >
                          {folded().has(sandbox.id) ? '▸' : '▾'}
                        </button>
                      </Show>
                      <input
                        type="checkbox"
                        checked={isSelected(sandbox.id)}
                        onChange={() => ui.selection.toggle('sandbox', sandbox.id)}
                        aria-label={`Select ${sandbox.name}`}
                      />
                    </td>
                    <td>
                      {/* The name opens the Admin Center, because that is
                          where the questions about a sandbox are answered. The
                          drawer stays one click away for a quick look. */}
                      <button
                        class="link"
                        onClick={() =>
                          props.onOpenAdmin ? props.onOpenAdmin(sandbox.id) : setOpenId(sandbox.id)
                        }
                      >
                        {sandbox.name}
                      </button>
                      <Show when={row.role === 'lead'}>
                        <span class="stack-summary">
                          <b>Stack</b> · {row.members.length} member{row.members.length === 1 ? '' : 's'}
                          {' · '}
                          <StatusBadge status={worstStatus(row.members.map((m) => m.status))} />
                        </span>
                      </Show>
                      <Show when={row.orphanOf}>
                        <span class="stack-summary">
                          part of {leadName(row.orphanOf) || 'a sandbox on another page'}
                        </span>
                      </Show>
                    </td>
                    <td class="small">
                      <StatusBadge status={sandbox.status} />
                      <Show when={sandbox.health && sandbox.health !== 'healthy'}>{' '}<StatusBadge status={sandbox.health} /></Show>
                      <span class="muted small" title="Uptime">{' '}{shortUptime(sandbox)}</span>
                    </td>
                    <td class="mono small" title="The agent this sandbox runs">{sandbox.pack_key}</td>
                    <td class="small" title="Who created it and manages it">{sandbox.owner_name || <span class="muted">shared</span>}</td>
                    <td class="mono small">
                      <Show when={sandbox.cluster_name}><span class="muted">{sandbox.cluster_name} › </span></Show>
                      {sandbox.gateway_name || <span class="muted">not placed</span>}
                      <Show when={sandbox.node_name}>
                        {' · '}
                        <Show when={props.onOpenNode} fallback={<span>{sandbox.node_name}</span>}>
                          <button class="link" title="Show this node" onClick={() => props.onOpenNode(sandbox.node_id)}>
                            {sandbox.node_name}
                          </button>
                        </Show>
                      </Show>
                    </td>
                    <td class="small">
                      <Show when={sandbox.supervisor_name} fallback={<span class="muted">none</span>}>
                        <span class="mono">{sandbox.supervisor_name}</span>{' '}
                        <StatusBadge status={sandbox.supervisor_status || 'unknown'} />
                      </Show>
                    </td>
                    <td class="small"><LoginBadge entry={logins()[sandbox.id]} /></td>
                    <td class="small">
                      <Show when={summaries()[sandbox.id]} fallback={<span class="muted">—</span>}>
                        <span
                          class={`badge ${summaries()[sandbox.id].egress_mode === 'off_allow_all' ? 'bad' : 'info'}`}
                          title="What the egress proxy does with this sandbox's traffic"
                        >
                          <i />{modeLabel(summaries()[sandbox.id].egress_mode)}
                        </span>
                        <Show when={summaries()[sandbox.id].allow_all}>
                          {' '}
                          <span class="badge bad" title="Everything this sandbox asks for is allowed right now">
                            <i />ALL ALLOWED
                          </span>{' '}
                          <Countdown until={summaries()[sandbox.id].allow_all_until} />
                        </Show>
                      </Show>
                    </td>
                    <td class="small">
                      <PresenceDots
                        people={(summaries()[sandbox.id]?.viewers || []).map((viewer) => ({
                          name: viewer.user_name || 'SSH',
                          kind: viewer.kind,
                          view_only: !!viewer.view_only,
                        }))}
                      />
                    </td>
                    <td class="small">
                      <span class={`runtime-tag ${sandbox.runtime_kind || 'unknown'}`}
                            title={sandbox.runtime_kind === 'openshell'
                              ? 'Enforced by an NVIDIA OpenShell supervisor (Landlock, seccomp, per-program network policy)'
                              : 'A plain Docker container without an OpenShell supervisor'}>
                        {sandbox.runtime_kind === 'openshell' ? 'OpenShell' : sandbox.runtime_kind || '—'}
                      </span>
                    </td>
                    <td class="actions">
                      <button class="tiny" title="How to reach this sandbox: host, ports, SSH"
                              onClick={() => setConnecting(sandbox)}>Connect</button>
                      <button class="tiny" title="The Workbox: live control of this sandbox"
                              onClick={() => navigate('/box/' + sandbox.id)}>Workbox</button>
                      <button class="tiny" onClick={() => setOpenId(sandbox.id)}>Details</button>
                      <Show when={state.canStop(sandbox)}
                            fallback={<button class="tiny" disabled={!state.canStart(sandbox)}
                                              onClick={() => state.act(sandbox.id, 'start')}>Start</button>}>
                        <button class="tiny" onClick={() => state.act(sandbox.id, 'stop')}>Stop</button>
                      </Show>
                      <details class="more-menu">
                        <summary class="tiny" title="Restart, stack, delete">More</summary>
                        <div class="more-pop">
                          <button class="tiny" disabled={!sandbox.status.includes('running')}
                                  onClick={() => state.act(sandbox.id, 'restart')}>Restart</button>
                          <Show when={row.role === 'lead'}>
                            <button class="tiny" title="Start every member and the lead"
                                    onClick={() => state.actAll([sandbox, ...row.members].filter(state.canStart).map((s) => s.id), 'start')}>
                              Start all
                            </button>
                            <button class="tiny" title="Stop every member and the lead"
                                    onClick={() => state.actAll([sandbox, ...row.members].filter(state.canStop).map((s) => s.id), 'stop')}>
                              Stop all
                            </button>
                          </Show>
                          <select
                            class="tiny stack-select"
                            aria-label={`Stack of ${sandbox.name}`}
                            title="Which stack this sandbox belongs to"
                            disabled={row.role === 'lead'}
                            value={sandbox.parent_id || ''}
                            onChange={(event) => state.setStack(sandbox.id, event.currentTarget.value)}
                          >
                            <option value="">{row.role === 'lead' ? 'Leads a stack' : 'No stack'}</option>
                            <For each={possibleLeads(state.items(), sandbox)}>
                              {(lead) => <option value={lead.id}>Part of {lead.name}</option>}
                            </For>
                          </select>
                          <button class="tiny danger" onClick={() => confirmDestroy(sandbox)}>Delete</button>
                        </div>
                      </details>
                    </td>
                  </tr>
  );

  return (
    <section class="page">
      <header class="page-head">
        <div>
          <h1>Sandboxes</h1>
          <p class="muted">
            {state.total()} total · {state.items().length} shown
            {state.status() ? ` · filtered by ${state.status()}` : ''}
          </p>
        </div>
        <div class="controls">
          <button class="primary" onClick={() => { setCreated(null); setCreating(true); }}>
            New sandbox
          </button>
          <input
            type="search"
            placeholder="Search by name…"
            value={state.search()}
            onInput={(event) => state.updateSearch(event.currentTarget.value)}
          />
          <button class="secondary" onClick={() => state.load({ cursor: null })} disabled={state.loading()}>
            {state.loading() ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>
      </header>
      <HostLoad />

      <p class="hierarchy muted" role="note">
        <b>How it fits together:</b> a <b>cluster</b> holds <b>gateways</b>; a gateway carries{' '}
        <b>sandboxes</b> and hands each its policy. Every sandbox has its own <b>supervisor</b>{' '}
        (a service, not a sandbox) that watches it from outside and decides each network{' '}
        connection. The agent runs inside the sandbox.
      </p>

      <Show when={created()}>
        <p class="notice">{created()}</p>
      </Show>

      <Show when={state.error()}>
        <p class="error">{state.error()}</p>
      </Show>

      <div class="facets">
        <For each={Object.entries(state.facetsStatus())}>
          {([value, count]) => (
            <button
              class={state.status() === value ? 'chip active' : 'chip'}
              onClick={() => state.filterByStatus(value)}
            >
              {value} <b>{count}</b>
            </button>
          )}
        </For>
      </div>

      <Show
        when={state.items().length > 0}
        fallback={
          <Empty
            title="No sandboxes yet"
            hint="Create one here, choosing a node and its share of that node's memory, CPU, and PIDs."
          />
        }
      >
        <div class="table-scroll">
          <table>
            <thead>
              <tr>
                <th class="narrow">
                  <input
                    type="checkbox"
                    checked={allSelected()}
                    onChange={() => (allSelected() ? ui.selection.clear() : ui.selection.set('sandbox', state.items().map((item) => item.id)))}
                    aria-label="Select all"
                  />
                </th>
                <th>Name</th>
                <th title="Whether it runs, how healthy it is, and for how long">Status</th>
                <th title="The agent the sandbox runs">Agent</th>
                <th title="The user who owns it. A user sees only their own sandboxes and the shared ones; an admin sees all.">Owner</th>
                <th title="Cluster › the gateway that carries it · the node (machine) it runs on">Cluster › gateway · node</th>
                <th title="The service that watches this sandbox from outside and enforces its policy. One per sandbox, not a sandbox itself.">Supervisor</th>
                <th title="How the agent in the sandbox is signed in: the gateway key, or your own account">Login</th>
                <th title="What the egress proxy does with this sandbox's traffic, and whether everything is allowed right now">Network</th>
                <th title="Who is connected to this sandbox right now: terminal, SSH, or desktop">Connected</th>
                <th title="What enforces the sandbox">Runtime</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              <For each={rows()}>
                {(row) => (
                  <>
                    {sandboxRow(row.sandbox, row)}
                    <Show when={row.role === 'lead' && !folded().has(row.sandbox.id)}>
                      <For each={row.members}>
                        {(member) => sandboxRow(member, { role: 'member', lead: row.sandbox })}
                      </For>
                    </Show>
                  </>
                )}
              </For>
            </tbody>
          </table>
        </div>

        <Show when={state.nextCursor()}>
          <div class="pager">
            <button class="secondary"
                    onClick={() => state.load({ cursor: state.nextCursor(), append: true })}
                    disabled={state.loading()}>
              Load more
            </button>
          </div>
        </Show>
      </Show>

      <SandboxDetail
        sandboxId={openId()}
        onClose={() => setOpenId(null)}
        onChanged={() => state.load({ cursor: null })}
      />

      <Drawer
        open={!!connecting()}
        title={`Connect to ${connecting()?.name || ''}`}
        subtitle="The address and ports to use from your own machine."
        onClose={() => setConnecting(null)}
      >
        <Show when={connecting()}>
          <ConnectPanel sandboxId={connecting().id} />
        </Show>
      </Drawer>

      {/* The form is mounted only while the drawer is open, so every open reads
          the node budgets again instead of showing the ones from last time. */}
      <Drawer
        open={creating()}
        title="New sandbox"
        subtitle="Its limits are checked against the node it lands on."
        onClose={() => setCreating(false)}
      >
        <Show when={creating()}>
          <CreateSandboxForm
            onCancel={() => setCreating(false)}
            onCreated={createdSandbox}
          />
        </Show>
      </Drawer>
    </section>
  );
}

function shortUptime(sandbox) {
  const seconds = sandbox.uptime_seconds;
  if (!seconds) return '—';
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86400)}d`;
}
