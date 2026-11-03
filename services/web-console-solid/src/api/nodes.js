// Nodes: the machines sandboxes and models run on.
//
// `list` answers the collection envelope (`{ "items": [...] }`) like every other
// collection, and the server computed each budget so the console never sums
// installs itself.

import { request } from './client.js';

/** Every node with its budget and sandbox count. */
export function list() {
  return request('nodes');
}

export function get(id) {
  return request(`nodes/${id}`);
}

/**
 * The sandboxes of one node.
 *
 * The answer is the sandbox page shape, so the drawer reuses the columns of the
 * inventory instead of learning a second list.
 */
export function sandboxes(id) {
  return request(`nodes/${id}/sandboxes`);
}

/**
 * Measure the node through its own runner.
 *
 * A node whose runner does not answer is `503 node.runtime_unavailable`; that is
 * a fact about the node, not a failure of the console, so callers show it.
 */
export function probe(id) {
  return request(`nodes/${id}/probe`, { method: 'POST' });
}

/**
 * Rename or describe a node.
 *
 * Only the display fields are writable: `name` is the identifier in URLs, and
 * kind, status, and `container_gpu` are measured, never typed.
 */
export function updateDisplay(id, displayName, description) {
  return request(`nodes/${id}/display`, {
    method: 'PUT',
    body: { display_name: displayName, description: description || null },
  });
}

/** Set the address and hostname clients dial on a node; empty clears a field. */
export function setEndpoint(id, body) {
  return request(`nodes/${id}/endpoint`, { method: 'PUT', body });
}

/** A one-time link that joins a machine to a site of this data center (ADR 0047). */
export function joinToken(nodeName, siteId) {
  return request('node-join-tokens', { method: 'POST', body: { node_name: nodeName, site_id: siteId } });
}

/** Remove a joined node that has nothing running on it. */
export function removeJoined(id) {
  return request(`nodes/${id}`, { method: 'DELETE' });
}
