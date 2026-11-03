// The platform around the sandboxes: the appliance's own containers, the
// OpenShell gateways, and one supervisor per sandbox.
//
// Three tables, because they are three different things with three different
// owners:
//
//   * platform containers are Compose services of this appliance. An admin may
//     change their CPU/memory/process limits live and restart the ones whose
//     restart does not cut the request itself.
//   * gateways are the OpenShell process that talks to the supervisor of every
//     sandbox. Its limits are set by its unit, not by this page, and the page
//     says so instead of offering a button that would do nothing.
//   * supervisors are containers the gateway creates, one per sandbox. They are
//     listed so an operator can see them next to the rest, but they are
//     recreated with their sandbox; the sandbox's own Supervisor tab is where
//     they are inspected.

import { For, Show, createSignal, onMount } from 'solid-js';
import StatusBadge from '../../components/StatusBadge.jsx';
import { formatBytes } from '../../components/Meter.jsx';
import * as system from '../../api/system.js';
import * as cluster from '../../api/cluster.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';

const MiB = 1024 * 1024;
const HELP = {
  memory: 'The hard memory limit in MiB. Docker stops the container when it goes above it; leave it empty to keep the current limit. The platform refuses anything below 64 MiB.',
  cpus: 'How many CPU cores this container may use, as a decimal (0.5 = half a core). Empty keeps the current limit.',
  pids: 'The highest number of processes and threads this container may have. It is the guard against a fork bomb; empty keeps the current value.',
  restart: 'A restart interrupts this component for a moment. Sandboxes keep running, but requests through it fail while it comes back.',
};

const stateTone = (state) => (state === 'running' ? 'ok' : state === 'restarting' ? 'warn' : 'err');

function ago(value) {
  if (!value) return '—';
  const started = new Date(value).getTime();
  if (Number.isNaN(started)) return '—';
  const seconds = Math.max(0, Math.round((Date.now() - started) / 1000));
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.round(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)}h`;
  return `${Math.round(seconds / 86400)}d`;
}

export default function SystemPage(props) {
  const [components, setComponents] = createSignal([]);
  const [gateways, setGateways] = createSignal([]);
  const [supervisors, setSupervisors] = createSignal([]);
  const [error, setError] = createSignal(null);
  const [notice, setNotice] = createSignal(null);
  const [editing, setEditing] = createSignal(null);
  const [form, setForm] = createSignal({ memory: '', cpus: '', pids: '' });
  const isAdmin = () => props.role === 'admin';

  async function load() {
    setError(null);
    try {
      const [platform, gatewayList, supervisorList] = await Promise.all([
        system.components(),
        cluster.gateways(),
        cluster.supervisors(),
      ]);
      // `/system/components` answers with the bare list; older builds wrapped it.
      setComponents(Array.isArray(platform) ? platform : platform.items || []);
      setGateways(gatewayList.items || []);
      setSupervisors(supervisorList.items || []);
    } catch (failure) {
      setError(failure.message);
    }
  }
  onMount(load);

  function beginEdit(row) {
    const limits = row.limits || {};
    setEditing(row.service);
    setForm({
      memory: limits.memory_limit_bytes ? String(Math.round(limits.memory_limit_bytes / MiB)) : '',
      cpus: limits.nano_cpus ? String(limits.nano_cpus / 1_000_000_000) : '',
      pids: limits.pids_limit ? String(limits.pids_limit) : '',
    });
    setNotice(null);
  }

  async function save(event) {
    event.preventDefault();
    const body = {};
    const memory = form().memory.trim();
    const cpus = form().cpus.trim();
    const pids = form().pids.trim();
    if (memory) body.memory_limit_bytes = Math.round(Number(memory) * MiB);
    if (cpus) body.nano_cpus = Math.round(Number(cpus) * 1_000_000_000);
    if (pids) body.pids_limit = Number(pids);
    if (!Object.keys(body).length) {
      setError('Name at least one limit. An empty change does nothing.');
      return;
    }
    try {
      await system.setLimits(editing(), body);
      setNotice(`Limits of ${editing()} applied to the running container.`);
      setEditing(null);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  async function restart(row) {
    if (!window.confirm(`Restart ${row.service}? ${HELP.restart}`)) return;
    setError(null);
    try {
      await system.restart(row.service);
      setNotice(`${row.service} is restarting.`);
      await load();
    } catch (failure) {
      setError(failure.message);
    }
  }

  return (
    <section class="page">
      <PageHeader
        title="System containers"
        subtitle={<>The components the console and the sandboxes run on. Sandboxes are listed on their own page; this is the platform beneath them, so a gateway or a supervisor is edited here and not by hand on the node.</>}
        actions={<>
          <button class="secondary" onClick={load}>Refresh</button>
          <HelpTip anchor="nodes" />
        </>}
      />
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={notice()}><p class="notice">{notice()}</p></Show>

      <h3>Platform containers</h3>
      <div class="table-scroll">
        <table class="mcp-table">
          <thead>
            <tr><th>Component</th><th>What it does</th><th>State</th><th>Up</th><th>CPU</th><th>Memory</th><th>Limits</th><th /></tr>
          </thead>
          <tbody>
            <For each={components()}>
              {(row) => (
                <tr>
                  <td><b>{row.service}</b><div class="muted small mono">{row.image || '—'}</div></td>
                  <td class="small">{row.description}{row.restarts ? <div class="muted small">restarted {row.restarts}×</div> : null}</td>
                  <td><StatusBadge status={stateTone(row.state)} label={row.health ? `${row.state} · ${row.health}` : row.state} /></td>
                  <td class="small">{ago(row.started_at)}</td>
                  <td class="small">{row.cpu_pct == null ? '—' : `${row.cpu_pct} %`}</td>
                  <td class="small">
                    {row.mem_bytes == null ? '—' : formatBytes(row.mem_bytes)}
                    <div class="muted small">{row.mem_limit_bytes ? `of ${formatBytes(row.mem_limit_bytes)}` : 'no limit'}</div>
                  </td>
                  <td class="small mono">
                    {(row.limits?.memory_limit_bytes ? `${formatBytes(row.limits.memory_limit_bytes)} mem` : 'no mem limit')}
                    <div class="muted small">
                      {row.limits?.nano_cpus ? `${row.limits.nano_cpus / 1_000_000_000} cores` : 'unlimited cores'}
                      {row.limits?.pids_limit ? ` · ${row.limits.pids_limit} pids` : ''}
                    </div>
                  </td>
                  <td class="actions">
                    <Show when={isAdmin()} fallback={<span class="muted small">admin only</span>}>
                      <button class="tiny" onClick={() => beginEdit(row)}>Edit limits</button>
                      <button class="tiny danger" onClick={() => restart(row)}>Restart</button>
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <Show when={isAdmin() && editing()}>
        <form class="editor" onSubmit={save}>
          <h3>Limits of {editing()}</h3>
          <div class="form-grid">
            <label class="field"><span>Memory limit (MiB)</span>
              <input type="number" min="64" value={form().memory} placeholder="1024"
                onInput={(e) => setForm({ ...form(), memory: e.currentTarget.value })} />
              <small class="muted">{HELP.memory}</small></label>
            <label class="field"><span>CPU cores</span>
              <input type="number" min="0.1" step="0.1" value={form().cpus} placeholder="1.5"
                onInput={(e) => setForm({ ...form(), cpus: e.currentTarget.value })} />
              <small class="muted">{HELP.cpus}</small></label>
            <label class="field"><span>Process limit</span>
              <input type="number" min="16" value={form().pids} placeholder="512"
                onInput={(e) => setForm({ ...form(), pids: e.currentTarget.value })} />
              <small class="muted">{HELP.pids}</small></label>
          </div>
          <div class="actions">
            <button type="submit" class="primary">Apply to the running container</button>
            <button type="button" class="secondary" onClick={() => setEditing(null)}>Cancel</button>
          </div>
        </form>
      </Show>

      <h3>Gateways</h3>
      <p class="muted small">
        The OpenShell gateway is a service of the node, not a container of this stack. Its CPU and memory
        are set by that unit; changing them from here would be a promise this page cannot keep.
      </p>
      <div class="table-scroll">
        <table class="dense">
          <thead><tr><th>Gateway</th><th>Node</th><th>State</th><th>Version</th><th>Sandboxes</th></tr></thead>
          <tbody>
            <For each={gateways()} fallback={<tr><td colspan="5" class="muted">No gateway reported.</td></tr>}>
              {(row) => (
                <tr>
                  <td><b>{row.name}</b></td>
                  <td class="small">{row.node_name || row.node_id || '—'}</td>
                  <td><StatusBadge status={row.status === 'ready' || row.state === 'running' ? 'ok' : 'warn'} label={row.status || row.state || 'unknown'} /></td>
                  <td class="small mono">{row.version || '—'}</td>
                  <td class="small">{row.sandbox_count ?? '—'}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>

      <h3>Supervisors</h3>
      <p class="muted small">
        One supervisor per sandbox, created and recreated with it. Its resources follow the sandbox, so they
        are edited on the sandbox's own Resources panel; this table is for seeing all of them at once.
      </p>
      <div class="table-scroll">
        <table class="dense">
          <thead><tr><th>Supervisor</th><th>Sandbox</th><th>State</th><th>Version</th><th>Policy</th></tr></thead>
          <tbody>
            <For each={supervisors()} fallback={<tr><td colspan="5" class="muted">No supervisor reported.</td></tr>}>
              {(row) => (
                <tr>
                  <td class="mono small">{row.name}</td>
                  <td class="small">{row.sandbox_name || (row.sandbox_id ? row.sandbox_id.slice(0, 8) : 'unbound')}</td>
                  <td><StatusBadge status={row.status === 'healthy' || row.state === 'running' ? 'ok' : 'warn'} label={row.status || row.state || 'unknown'} /></td>
                  <td class="small mono">{row.version || '—'}</td>
                  <td class="small mono">{row.policy_version || row.reported?.policy_version || '—'}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </section>
  );
}
