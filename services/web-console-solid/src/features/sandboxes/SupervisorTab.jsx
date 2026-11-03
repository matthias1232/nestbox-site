// The supervisor of one sandbox: what it is, what it enforces, what it logged.
//
// On OpenShell every sandbox has a supervisor of its own. It is not a sandbox: it is
// a service that sits outside the boundary, checks every connection the agent makes
// against the policy, and holds the credentials the agent never sees. This tab says
// that in words first, then shows the facts.

import { For, Show, createSignal } from 'solid-js';
import * as deck from '../../api/deck.js';
import { usePolling } from '../../components/Poll.js';
import StatusBadge from '../../components/StatusBadge.jsx';

function endpointText(endpoint) {
  const host = endpoint.host || (endpoint.allowed_ips || []).join(', ') || '*';
  const port = endpoint.port ? `:${endpoint.port}` : endpoint.ports ? `:${endpoint.ports.join('/')}` : '';
  const how = endpoint.protocol ? ` ${endpoint.protocol}` : '';
  return `${host}${port}${how}`;
}

export default function SupervisorTab(props) {
  const [view, setView] = createSignal(null);
  const [error, setError] = createSignal(null);
  const poll = usePolling(() => deck.supervisor(props.sandbox.id), {
    everyMs: 6000,
    onData: (data) => { setView(data); setError(null); },
    onError: (failure) => setError(failure.message),
  });

  const supervisor = () => view()?.supervisor;
  const live = () => view()?.openshell;
  const policy = () => live()?.policy || {};
  const rules = () => Object.entries(policy().network_policies || {});

  return (
    <section class="tab-panel supervisor-tab">
      <p class="explain">
        <b>What a supervisor is.</b> The agent runs inside the sandbox and has no network of its own.
        Its <b>supervisor</b> runs beside it, outside the boundary: it decides every network connection
        against the policy, tells the gateway what happened, and hands out credentials only to approved
        destinations. If the supervisor loses the gateway, the agent is frozen. The supervisor is a
        service, not a sandbox.
      </p>

      <div class="controls">
        <button class={poll.paused() ? 'secondary tiny' : 'primary tiny'} onClick={poll.toggle}>
          {poll.paused() ? 'Paused' : 'Live'}
        </button>
        <button class="secondary tiny" onClick={poll.refresh}>Refresh</button>
      </div>

      <Show when={error()}>
        <p class="error">{error()}</p>
        <p class="muted small">
          Only a sandbox that runs on an OpenShell gateway has an OpenShell supervisor.
        </p>
      </Show>

      <Show when={view()?.openshell_note}>
        <p class="notice">{view().openshell_note}</p>
      </Show>

      <Show when={view()}>
        <div class="supervisor-grid">
          <div class="card">
            <h3>Supervisor</h3>
            <Show when={supervisor()} fallback={<p class="muted">No supervisor is recorded for this sandbox yet.</p>}>
              <dl class="facts">
                <dt>Name</dt><dd class="mono">{supervisor().name}</dd>
                <dt>Status</dt><dd><StatusBadge status={supervisor().status} /></dd>
                <dt>Version</dt><dd class="mono">{supervisor().version || '—'}</dd>
                <dt>Last report</dt><dd>{supervisor().last_seen ? new Date(supervisor().last_seen).toLocaleTimeString() : 'never'}</dd>
                <dt>Pending configuration</dt><dd>{supervisor().config_dirtiness}</dd>
              </dl>
            </Show>
          </div>

          <div class="card">
            <h3>Boundary</h3>
            <dl class="facts">
              <dt>Phase</dt><dd><StatusBadge status={(live()?.sandbox?.phase || 'unknown').toLowerCase()} /></dd>
              <dt>Gateway</dt><dd class="mono">OpenShell {live()?.gateway_version || '—'}</dd>
              <dt>Policy version</dt><dd class="mono">{live()?.sandbox?.current_policy_version ?? '—'}</dd>
              <dt>Policy source</dt><dd>{live()?.sandbox?.policy_source ?? '—'}</dd>
              <dt>Restart policy</dt><dd>{live()?.sandbox?.restart_policy ?? '—'} ({live()?.sandbox?.restart_count ?? 0} restarts)</dd>
              <dt>Configuration</dt><dd>{live()?.sandbox?.configuration_admission?.state ?? '—'}</dd>
            </dl>
            <ul class="conditions">
              <For each={live()?.sandbox?.conditions || []}>
                {(item) => (
                  <li>
                    <StatusBadge status={item.status === 'True' ? 'ok' : 'warn'} /> {item.type}
                    <Show when={item.message}><span class="muted small"> — {item.message}</span></Show>
                  </li>
                )}
              </For>
            </ul>
          </div>
        </div>

        <div class="card">
          <h3>What it enforces</h3>
          <p class="muted small">
            Files are confined by Landlock, connections by the supervisor. Everything not listed is denied.
          </p>
          <div class="supervisor-grid">
            <div>
              <h4>Files the agent may read</h4>
              <p class="mono small">{(policy().filesystem_policy?.read_only || []).join('  ') || '—'}</p>
              <h4>Files the agent may write</h4>
              <p class="mono small">{(policy().filesystem_policy?.read_write || []).join('  ') || '—'}</p>
            </div>
            <div>
              <h4>Connections allowed ({rules().length} rules)</h4>
              <Show when={rules().length > 0} fallback={<p class="muted small">None: every connection is denied.</p>}>
                <ul class="rules">
                  <For each={rules()}>
                    {([key, rule]) => (
                      <li>
                        <b>{rule.name || key}</b>
                        <div class="mono small">{(rule.endpoints || []).map(endpointText).join(' · ')}</div>
                        <div class="muted small">for {(rule.binaries || []).map((b) => b.path).join(', ') || 'any program'}</div>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </div>
          </div>
        </div>

        <div class="card">
          <h3>Recent supervisor and gateway log</h3>
          <pre class="log">{(live()?.logs || []).join('\n') || 'No log lines yet.'}</pre>
        </div>
      </Show>
    </section>
  );
}
