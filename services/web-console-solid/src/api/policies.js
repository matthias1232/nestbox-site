// Policies, network rules, drafts, and temporary windows.

import { request, query } from './client.js';

export function list() {
  return request('policies');
}

export function get(id) {
  return request(`policies/${id}`);
}

export function create(body) {
  return request('policies', { method: 'POST', body });
}

export function update(id, body) {
  return request(`policies/${id}`, { method: 'PUT', body });
}

export function remove(id) {
  return request(`policies/${id}`, { method: 'DELETE' });
}

export function diff(id, against) {
  return request(`policies/${id}/diff${query({ against })}`);
}

export function networkRules() {
  return request('network-rules');
}

export function createRule(body) {
  return request('network-rules', { method: 'POST', body });
}

export function removeRule(id) {
  return request(`network-rules/${id}`, { method: 'DELETE' });
}

export function drafts() {
  return request('rule-drafts');
}

export function allowDestination(sandboxId, body) {
  return request(`sandboxes/${sandboxId}/allow`, { method: 'POST', body });
}

export function draftFromEvent(event, sandboxId) {
  return request('rule-drafts', {
    method: 'POST',
    body: { source_kind: 'network', event, sandbox_id: sandboxId },
  });
}

export function decideDraft(id, approve, reason) {
  const action = approve ? 'approve' : 'reject';
  return request(`rule-drafts/${id}/${action}`, { method: 'POST', body: { reason } });
}

// The revision history, newest first: what changed, when, and by whom.
export function versions(policyId) {
  return request(`policies/${policyId}/versions`);
}

// The windows in force: mode, channel and expiry, for the master switches.
export function windows() {
  return request('traffic-windows');
}

// `mode` is `bypass` (allow quietly) or `audit` (allow and record the block).
// `channel` is `all`, `proxy`, `dns`, `mcp` or `gateway`; `openshell` is refused
// by the server because its supervisor carries the sandbox's tooling.
export function openWindow(sandboxId, minutes, reason, mode = 'bypass', channel = 'all', unlimited = false) {
  return request('traffic-windows', {
    method: 'POST',
    body: { sandbox_id: sandboxId, minutes, unlimited, reason, mode, channel },
  });
}

// One allow draft per destination that was denied or only observed.
export function bulkDrafts(body) {
  return request('rule-drafts/bulk', { method: 'POST', body });
}

export function closeWindow(id) {
  return request(`traffic-windows/${id}`, { method: 'DELETE' });
}

// The sections and fields a policy document may contain; the forms render from this.
export function schema() {
  return request('policy-schema');
}

export function templates() {
  return request('policy-templates');
}

export function instantiateTemplate(id, assignTo = null) {
  return request(`policy-templates/${id}/instantiate`, { method: 'POST', body: { assign_to: assignTo } });
}

export function dnsRules() {
  return request('dns-rules');
}

export function createDnsRule(body) {
  return request('dns-rules', { method: 'POST', body });
}

export function removeDnsRule(id) {
  return request(`dns-rules/${id}`, { method: 'DELETE' });
}

export function sandboxPolicy(sandboxId) {
  return request(`sandboxes/${sandboxId}/policy`);
}

export function assignPolicy(sandboxId, policyId) {
  return request(`sandboxes/${sandboxId}/policy`, { method: 'PUT', body: { policy_id: policyId } });
}

export function unassignPolicy(sandboxId) {
  return request(`sandboxes/${sandboxId}/policy`, { method: 'DELETE' });
}

export function sandboxRules(sandboxId) {
  return request(`sandboxes/${sandboxId}/rules`);
}

export function egressMode(sandboxId) {
  return request(`sandboxes/${sandboxId}/egress-mode`);
}

export function setEgressMode(sandboxId, mode, revertAfterMinutes = null) {
  return request(`sandboxes/${sandboxId}/egress-mode`, {
    method: 'PUT',
    body: { mode, revert_after_minutes: revertAfterMinutes },
  });
}

// Copy a policy under a new name (never as a global override).
export function clonePolicy(id, name) {
  return request(`policies/${id}/clone`, { method: 'POST', body: { name } });
}

// Assign one policy to several sandboxes at once.
export function assignMany(id, sandboxIds) {
  return request(`policies/${id}/assign`, { method: 'POST', body: { sandbox_ids: sandboxIds } });
}

// Copy rules to other sandboxes, or make them apply to every sandbox.
export function copyRules(body) {
  return request('network-rules/copy', { method: 'POST', body });
}

// A rule with an end: allow one destination for 15 min, 1 h, 24 h or forever.
export function quickAllow(sandboxId, host, port, minutes) {
  return request(`sandboxes/${sandboxId}/quick-allow`, {
    method: 'POST',
    body: { host, port: port ?? null, minutes: minutes ?? null },
  });
}

export function quickAllowList(sandboxId) {
  return request(`sandboxes/${sandboxId}/quick-allow`);
}

// Compile a spec that is not saved yet: `{ spec, sandbox_id }` (contract C11).
export function preview(body) {
  return request('policies/preview', { method: 'POST', body });
}

// The compiled OpenShell document of a stored policy.
export function compiled(id) {
  return request(`policies/${id}/compiled`);
}
