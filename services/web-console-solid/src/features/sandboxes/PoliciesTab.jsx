// Policies tab of the sandbox Admin Center.
//
// Everything that decides what this sandbox may do on the network, in one place:
// the policy it carries (and a way to change it), how deeply its egress is
// observed, the rules that apply to it right now, a form for a rule of its own,
// and a temporary window for "let it through for fifteen minutes".

import { For, Show, createResource, createSignal } from 'solid-js';
import * as api from '../../api/policies.js';
import * as proxyApi from '../../api/proxies.js';
import StatusBadge from '../../components/StatusBadge.jsx';
import { NetworkRuleForm, DnsRuleForm } from '../policies/RuleForms.jsx';
import { MODES as NETWORK_MODES } from '../workbox/network.js';

// What each mode means here, where an operator reads before choosing. The labels
// come from `features/workbox/network.js`, so the user's card and this admin tab
// never call the same mode by two different names.
const NOTES = {
  transparent: 'The proxy opens every encrypted connection and reads the request, so methods, paths, bodies and chats can be matched and recorded. The node CA is mounted into this sandbox only, and its tools are told to trust it.',
  hybrid: 'Only connections to model providers (Claude, Codex / ChatGPT, OpenAI, Gemini, OpenRouter and the like — the ones an agent signed in with an account reaches directly, where no key can be put on the request) are opened and recorded, with chats and token use. Everything else stays an unread tunnel the proxy only allows or blocks by host. Add your own gateway with NESTBOX_AGENT_HOSTS.',
  forward_proxy: 'Every request goes through the node\'s egress proxy, which sees the destination and decides per request. It does not read inside an encrypted connection, so no CA is needed and no chat or token use can be shown.',
  off_allow_all: 'Nothing is blocked and nothing is opened: the proxy records host, port and size, and OpenShell allows everything. What the rules would have blocked is still recorded, so it can be turned into drafts.',
};

const MODES = [
  ...NETWORK_MODES.map((item) => ({ value: item.value, label: item.label, note: NOTES[item.value] })),
  {
    value: 'direct',
    label: 'Direct — no proxy (rebuilds the sandbox)',
    note: 'The sandbox leaves on its own route: no proxy in the environment, no CA, nothing observed. Policy rules still exist, but this traffic does not pass the proxy that enforces them — choose it only when the sandbox must not be observed at all. Runs on the Docker runtime, because a gateway sandbox has no route of its own.',
  },
];

export default function PoliciesTab(props) {
  const id = () => props.sandbox?.id;
  const [policies] = createResource(() => api.list());
  const [assignment, { refetch: refetchAssignment }] = createResource(id, (value) => api.sandboxPolicy(value));
  const [rules, { refetch: refetchRules }] = createResource(id, (value) => api.sandboxRules(value));
  const [mode, { refetch: refetchMode }] = createResource(id, (value) => api.egressMode(value).catch(() => null));
  const [exits] = createResource(() => proxyApi.list().catch(() => ({ items: [] })));
  const [exit, { refetch: refetchExit }] = createResource(id, (value) =>
    proxyApi.sandboxProxy(value).catch(() => null));
  const [chosen, setChosen] = createSignal('');
  const [message, setMessage] = createSignal(null);
  const [failure, setFailure] = createSignal(null);
  const [minutes, setMinutes] = createSignal(15);
  const [reason, setReason] = createSignal('');

  async function run(action, done, after) {
    setFailure(null);
    setMessage(null);
    try {
      await action();
      if (done) setMessage(done);
      await after?.();
      props.reload?.();
    } catch (error) {
      setFailure(error.message);
    }
  }

  const current = () => assignment()?.assignment;
  const currentModeValue = () => mode()?.mode || 'forward_proxy';
  const currentExit = () => exit()?.proxy?.id || '';
  const exitList = () => (exits()?.items || []).filter((item) => item.enabled);

  return (
    <section class="tab-panel">
      <Show when={failure()}><p class="error">{failure()}</p></Show>
      <Show when={message()}><p class="notice">{message()}</p></Show>

      <h3>Policy</h3>
      <p class="small">
        <Show when={current()} fallback={<span class="muted">No policy is assigned. The sandbox starts with the safe defaults, but nothing here can be tuned until one is assigned.</span>}>
          Carrying <b>{current().policy_name || current().name || current().policy_id}</b>
        </Show>
      </p>
      <div class="controls">
        <select value={chosen()} onChange={(e) => setChosen(e.currentTarget.value)}>
          <option value="">choose a policy…</option>
          <For each={policies()?.items || []}>{(item) => <option value={item.id}>{item.name}</option>}</For>
        </select>
        <button class="primary tiny" disabled={!chosen()}
          onClick={() => run(() => api.assignPolicy(id(), chosen()), 'The policy is assigned. The network section applies to the running sandbox at once.', refetchAssignment)}>
          Assign
        </button>
        <Show when={current()}>
          <button class="secondary tiny" onClick={() => run(() => api.unassignPolicy(id()), 'The policy is removed.', refetchAssignment)}>Remove</button>
        </Show>
      </div>

      <h3>How deeply is its egress observed?</h3>
      <div class="controls">
        <select value={currentModeValue()} onChange={(e) => run(async () => {
          const saved = await api.setEgressMode(id(), e.currentTarget.value);
          // A sandbox made from a recipe is made again, and the new one has a new id.
          if (saved?.sandbox_id && saved.sandbox_id !== id()) props.followed?.(saved.sandbox_id);
        }, 'Mode saved; it applies to new connections within seconds.', refetchMode)}>
          <For each={MODES}>{(item) => <option value={item.value}>{item.label}</option>}</For>
        </select>
      </div>
      <p class="muted small">{MODES.find((item) => item.value === currentModeValue())?.note}</p>
      <p class="muted small">
        Modes apply live. Only 'Direct' rebuilds the sandbox.
      </p>

      <h3>Which exit does it leave through?</h3>
      <div class="controls">
        <select value={currentExit()} onChange={(e) => run(
          () => proxyApi.setSandboxProxy(id(), e.currentTarget.value || null),
          'Exit saved. A running sandbox is recreated so the change applies.',
          refetchExit,
        )}>
          <option value="">Node's own route</option>
          <For each={exitList()}>
            {(item) => <option value={item.id}>{item.name}{item.country ? ` · ${item.country}` : ''}</option>}
          </For>
        </select>
      </div>
      <Show
        when={currentModeValue() === 'direct'}
        fallback={
          <p class="muted small">
            An exit applies to <b>Direct</b> egress. In the proxied modes the node's egress
            proxy is the exit, because that is the point where rules are enforced; switch to
            Direct to route this sandbox through the chosen exit.
          </p>
        }
      >
        <p class="muted small">
          Direct with an exit sends the sandbox's traffic to that proxy and nothing else
          inspects it. Without an exit it uses the node's own route.
        </p>
      </Show>

      <h3>Rules that apply to this sandbox</h3>
      <Show when={(rules()?.items || []).length > 0} fallback={<p class="muted small">No rule applies. With default deny, nothing leaves this sandbox.</p>}>
        <div class="table-scroll">
          <table>
            <thead><tr><th>Action</th><th>Name</th><th>Conditions</th><th>Enforcement</th><th class="narrow">Priority</th><th /></tr></thead>
            <tbody>
              <For each={rules().items}>
                {(rule) => (
                  <tr>
                    <td><StatusBadge status={rule.action === 'allow' ? 'running' : 'denied'} label={rule.action} /></td>
                    <td class="small">{rule.name}</td>
                    <td class="mono small">
                      {(rule.matchers || []).map((m) => (m.value ? `${m.kind}=${m.value}` : m.kind)).join(' · ') || `${rule.host}${rule.port ? ':' + rule.port : ''}`}
                    </td>
                    <td class="small">{rule.enforcement}</td>
                    <td class="narrow mono">{rule.priority}</td>
                    <td>
                      <Show when={rule.sandbox_id}>
                        <button class="tiny" onClick={() => run(() => api.removeRule(rule.id), 'Rule deleted.', refetchRules)}>Delete</button>
                      </Show>
                    </td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </div>
      </Show>

      <h3>Let it through for a while</h3>
      <p class="muted small">Opens a temporary window in which this sandbox's traffic is allowed and recorded. It closes by itself.</p>
      <div class="controls">
        <input type="number" min="1" max="240" value={minutes()} onInput={(e) => setMinutes(Number(e.currentTarget.value))} style="width:80px" />
        <span class="small muted">minutes</span>
        <input placeholder="why (required)" value={reason()} onInput={(e) => setReason(e.currentTarget.value)} />
        <button class="secondary tiny" disabled={!reason().trim()}
          onClick={() => run(() => api.openWindow(id(), minutes(), reason().trim()), 'The window is open.')}>
          Open window
        </button>
      </div>

      <NetworkRuleForm sandboxId={id()} onCreated={() => { setMessage('Rule created.'); refetchRules(); }} />
      <DnsRuleForm sandboxId={id()} onCreated={() => { setMessage('DNS rule created.'); }} />
    </section>
  );
}
