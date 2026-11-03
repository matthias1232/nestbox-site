// Pure logic of the policy wizard: no JSX, no network. The wizard keeps one `state`
// object (`newState`); every step edits the `spec` inside it through the helpers
// below, and `applyPlan` turns the state into the ordered calls the Review step
// lists and Apply runs.

import { splitList } from './policyForms.js';

// The group the wizard puts its allow entries into.
export const ALLOW_GROUP = 'wizard-allow';

// The egress modes with the one sentence the Observation step shows for each.
export const EGRESS_MODES = [
  { value: 'forward_proxy', label: 'Forward proxy', text: 'Every request goes through the node egress proxy, which decides per request. Rules are enforced.' },
  { value: 'transparent', label: 'Transparent', text: 'Traffic is intercepted without proxy settings in the sandbox, so tools that ignore proxy variables are still covered.' },
  { value: 'hybrid', label: 'Hybrid', text: 'Transparent interception for everything plus the forward proxy for tools that ask for it.' },
  { value: 'off_allow_all', label: 'Observe only', text: 'Nothing is blocked; the proxy records what the rules would have blocked so it can become drafts.' },
];

// A document with nothing allowed: a sandbox with it can reach nothing.
export function emptySpec() {
  return {
    filesystem_policy: { paths: [{ path: '/workspace', access: 'read_write' }] },
    network_policies: {},
    process: { run_as_user: 'sandbox', max_processes: 64 },
  };
}

// The starting state of the wizard; `policy` is a stored policy when editing.
export function newState({ policy = null, sandboxId = null } = {}) {
  return {
    policyId: policy?.id ?? null,
    version: policy?.version ?? null,
    name: policy?.name ?? '',
    scope: policy?.scope ?? 'sandbox',
    spec: policy?.spec ? structuredClone(policy.spec) : emptySpec(),
    dns: [],
    sandboxId,
    egressMode: null,
    windowMinutes: 0,
  };
}

// The steps, in order. The Tools step needs a sandbox to say what is attached.
export function stepsFor(sandboxId) {
  const steps = [
    { id: 'network', label: 'Network' },
    { id: 'dns', label: 'DNS' },
    { id: 'filesystem', label: 'Filesystem' },
    { id: 'process', label: 'Process and Landlock' },
  ];
  if (sandboxId) steps.push({ id: 'tools', label: 'Tools and MCP' });
  steps.push({ id: 'observation', label: 'Observation' }, { id: 'review', label: 'Review' });
  return steps;
}

// Every endpoint of every network group as one flat list, in document order.
export function entries(spec) {
  const out = [];
  for (const [group, body] of Object.entries(spec?.network_policies || {})) {
    for (const [index, endpoint] of (body?.endpoints || []).entries()) out.push({ group, index, endpoint });
  }
  return out;
}

// Add one allowed destination. The same host, port and protocol is added once.
export function addAllow(spec, { host, port, protocol, methods } = {}) {
  const cleanHost = String(host ?? '').trim();
  if (!cleanHost) return spec;
  const endpoint = { host: cleanHost, port: port ? Number(port) : 443, protocol: protocol || 'rest', access: 'allow' };
  if (methods && methods.length) endpoint.methods = methods.map((m) => String(m).toUpperCase());
  const exists = entries(spec).some(
    (e) => e.endpoint.host === endpoint.host && e.endpoint.port === endpoint.port && e.endpoint.protocol === endpoint.protocol,
  );
  if (exists) return spec;
  const groups = spec.network_policies || {};
  const group = groups[ALLOW_GROUP] || { enforcement: 'enforced', endpoints: [] };
  return {
    ...spec,
    network_policies: { ...groups, [ALLOW_GROUP]: { ...group, endpoints: [...(group.endpoints || []), endpoint] } },
  };
}

// Replace one endpoint by its flat index (see `entries`).
export function updateEntry(spec, i, patch) {
  const target = entries(spec)[i];
  if (!target) return spec;
  const group = spec.network_policies[target.group];
  const endpoints = group.endpoints.map((e, k) => (k === target.index ? { ...e, ...patch } : e));
  return { ...spec, network_policies: { ...spec.network_policies, [target.group]: { ...group, endpoints } } };
}

// Remove one endpoint by its flat index; a group left empty is dropped.
export function removeEntry(spec, i) {
  const target = entries(spec)[i];
  if (!target) return spec;
  const group = spec.network_policies[target.group];
  const endpoints = group.endpoints.filter((_, k) => k !== target.index);
  const groups = { ...spec.network_policies };
  if (endpoints.length === 0) delete groups[target.group];
  else groups[target.group] = { ...group, endpoints };
  return { ...spec, network_policies: groups };
}

// The DNS list of the wizard: `[{ action: 'allow' | 'deny', pattern }]`, cleaned
// and without duplicates. DNS rules are not part of the spec; they are saved as
// DNS rules of the policy.
export function setDns(dns) {
  const seen = new Set();
  const out = [];
  for (const item of dns || []) {
    const pattern = String(item.pattern ?? '').trim().toLowerCase();
    const action = item.action === 'allow' ? 'allow' : 'deny';
    const key = `${action}|${pattern}`;
    if (!pattern || /\s/.test(pattern) || seen.has(key)) continue;
    seen.add(key);
    out.push({ action, pattern });
  }
  return out;
}

// Set the process fields; an empty value removes the key. `binaries` and
// `env_allow` take a list or a comma/newline separated text.
export function setProcess(spec, fields) {
  const next = { ...(spec.process || {}) };
  for (const [key, raw] of Object.entries(fields || {})) {
    let value = raw;
    if (key === 'max_processes') {
      const number = Number(raw);
      value = raw === '' || raw === null || raw === undefined || !Number.isFinite(number) ? undefined : Math.trunc(number);
    } else if (key === 'binaries' || key === 'env_allow') {
      const list = Array.isArray(raw) ? raw : splitList(raw);
      value = list.length ? list : undefined;
    } else {
      const text = String(raw ?? '').trim();
      value = text === '' ? undefined : text;
    }
    if (value === undefined) delete next[key];
    else next[key] = value;
  }
  return { ...spec, process: next };
}

// The Landlock rule list of a spec, `[{ path, access: [] }]`.
export function landlockRules(spec) {
  return spec?.landlock?.rules || [];
}

// Replace the Landlock rules; none removes the whole section.
export function setLandlock(spec, rules) {
  const clean = (rules || [])
    .map((r) => ({ path: String(r.path ?? '').trim(), access: r.access || [] }))
    .filter((r) => r.path);
  const next = { ...spec };
  if (clean.length === 0) delete next.landlock;
  else next.landlock = { rules: clean };
  return next;
}

// The filesystem read-only / read-write lists as one-path-per-line text.
export function filesystemLists(spec) {
  const paths = spec?.filesystem_policy?.paths || [];
  const pick = (access) => paths.filter((p) => p.access === access).map((p) => p.path);
  return { read_only: pick('read_only'), read_write: pick('read_write') };
}

// Rebuild the filesystem section from two path lists (arrays or text). Other
// access kinds already in the document (for example `execute`) are kept.
export function setFilesystemLists(spec, { read_only, read_write }) {
  const asList = (v) => (Array.isArray(v) ? v : String(v ?? '').split('\n')).map((p) => p.trim()).filter(Boolean);
  const kept = (spec?.filesystem_policy?.paths || []).filter((p) => p.access !== 'read_only' && p.access !== 'read_write');
  const paths = [
    ...asList(read_only).map((path) => ({ path, access: 'read_only' })),
    ...asList(read_write).map((path) => ({ path, access: 'read_write' })),
    ...kept,
  ];
  return { ...spec, filesystem_policy: { ...(spec.filesystem_policy || {}), paths } };
}

// What is wrong with a step, in one sentence, or null when it may proceed.
export function validateStep(step, spec, state = {}) {
  if (step === 'network') {
    for (const { endpoint } of entries(spec)) {
      if (!String(endpoint.host ?? '').trim()) return 'Every network entry needs a host.';
      const port = Number(endpoint.port);
      if (endpoint.port !== undefined && endpoint.port !== '' && (!Number.isInteger(port) || port < 1 || port > 65535)) {
        return `Port ${endpoint.port} is not between 1 and 65535.`;
      }
    }
    return null;
  }
  if (step === 'filesystem') {
    const paths = spec?.filesystem_policy?.paths || [];
    for (const p of paths) {
      if (!String(p.path).startsWith('/')) return `The path ${p.path} must be absolute.`;
      if (String(p.path).split('/').includes('..')) return `The path ${p.path} must not contain "..".`;
    }
    return null;
  }
  if (step === 'process') {
    const max = spec?.process?.max_processes;
    if (max !== undefined && (!Number.isInteger(max) || max < 1 || max > 4096)) return 'Max processes is between 1 and 4096.';
    for (const rule of landlockRules(spec)) {
      if (!String(rule.path).startsWith('/')) return `The Landlock path ${rule.path} must be absolute.`;
    }
    return null;
  }
  if (step === 'review') {
    if (!String(state.name ?? '').trim()) return 'Give the policy a name.';
    return null;
  }
  return null;
}

// The ordered calls Apply makes. Each item is `{ label, call }`, where `call` is
// `{ op, ...params }`; `$policy` in a param stands for the id of the policy saved by
// the first call. `runPlan` executes it; the Review step only prints the labels.
export function applyPlan(state) {
  const plan = [];
  const body = { name: String(state.name).trim(), scope: state.scope, spec: state.spec };
  if (state.policyId) {
    plan.push({
      label: `Save policy "${body.name}" (version ${state.version})`,
      call: { op: 'update', id: state.policyId, body: { spec: state.spec, version: state.version } },
    });
  } else {
    plan.push({ label: `Create policy "${body.name}" (${state.scope} scope)`, call: { op: 'create', body } });
  }
  for (const rule of state.dns || []) {
    plan.push({
      label: `${rule.action === 'allow' ? 'Allow' : 'Block'} DNS name ${rule.pattern}`,
      call: { op: 'dnsRule', form: { policy_id: '$policy', action: rule.action, pattern: rule.pattern, priority: 100 } },
    });
  }
  if (state.sandboxId) {
    plan.push({ label: 'Assign the policy to the sandbox', call: { op: 'assign', sandboxId: state.sandboxId, policyId: '$policy' } });
    if (state.egressMode) {
      plan.push({ label: `Set the egress mode to ${state.egressMode}`, call: { op: 'egress', sandboxId: state.sandboxId, mode: state.egressMode } });
    }
    if (state.windowMinutes > 0) {
      plan.push({
        label: `Open an evaluation window for ${state.windowMinutes} min`,
        call: { op: 'window', sandboxId: state.sandboxId, minutes: Number(state.windowMinutes) },
      });
    }
  }
  return plan;
}

// Run a plan in order. `ops` maps an op name to `async (call, ctx) => result`;
// the `create` and `update` ops must return the saved policy so its id can replace
// `$policy`. Stops at the first failure. Returns `{ results, policyId, failed }`
// where each result is `{ label, ok, message?, value? }`.
export async function runPlan(plan, ops) {
  const ctx = { policyId: null };
  const results = [];
  for (const item of plan) {
    const call = JSON.parse(JSON.stringify(item.call), (_k, v) => (v === '$policy' ? ctx.policyId : v));
    try {
      const value = await ops[call.op](call, ctx);
      if (call.op === 'create' || call.op === 'update') ctx.policyId = value?.policy?.id ?? value?.id ?? call.id ?? ctx.policyId;
      results.push({ label: item.label, ok: true, value });
    } catch (failure) {
      results.push({ label: item.label, ok: false, message: failure?.message || String(failure) });
      return { results, policyId: ctx.policyId, failed: true };
    }
  }
  return { results, policyId: ctx.policyId, failed: false };
}

// Suggestions from the destinations a sandbox reached: allowed-not-yet entries,
// one per host and port, most-requested first. `decisions` is `traffic.decisions().items`.
export function suggestionsFrom(decisions, spec) {
  const have = new Set(entries(spec).map((e) => `${e.endpoint.host}|${e.endpoint.port}`));
  const seen = new Map();
  for (const d of decisions || []) {
    if (!d.host || String(d.protocol).toLowerCase() === 'dns') continue;
    const port = d.port || 443;
    const key = `${d.host}|${port}`;
    if (have.has(key)) continue;
    const row = seen.get(key) || { host: d.host, port, protocol: d.protocol || 'rest', count: 0, decision: d.decision };
    row.count += Number(d.count) || 1;
    seen.set(key, row);
  }
  return [...seen.values()].sort((a, b) => b.count - a.count);
}

// DNS suggestions: names seen in DNS decisions that have no rule in the list yet.
export function dnsSuggestionsFrom(decisions, dns) {
  const have = new Set((dns || []).map((r) => r.pattern));
  const seen = new Map();
  for (const d of decisions || []) {
    if (String(d.protocol).toLowerCase() !== 'dns' || !d.host || have.has(d.host.toLowerCase())) continue;
    const row = seen.get(d.host) || { host: d.host, count: 0, decision: d.decision };
    row.count += Number(d.count) || 1;
    seen.set(d.host, row);
  }
  return [...seen.values()].sort((a, b) => b.count - a.count);
}
