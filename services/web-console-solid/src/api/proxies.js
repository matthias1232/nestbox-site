// Exit proxies: where a sandbox or a provider call leaves the node.
//
// The catalogue is data, not configuration: the firewall's exits are seeded once
// and every selector reads the same rows. Choosing an exit for a sandbox is a
// separate call from choosing its egress mode, because the two answer different
// questions (where it leaves, and how much of it is observed).

import { request } from './client.js';

export function list() {
  return request('proxies');
}

export function create(body) {
  return request('proxies', { method: 'POST', body });
}

export function update(id, body) {
  return request(`proxies/${id}`, { method: 'PUT', body });
}

export function remove(id) {
  return request(`proxies/${id}`, { method: 'DELETE' });
}

// A sandbox's exit. `proxyId` of null puts it back on the node's own route.
export function sandboxProxy(sandboxId) {
  return request(`sandboxes/${sandboxId}/proxy`);
}

export function setSandboxProxy(sandboxId, proxyId) {
  return request(`sandboxes/${sandboxId}/proxy`, { method: 'PUT', body: { proxy_id: proxyId } });
}

export function setProviderProxy(providerId, proxyId) {
  return request(`providers/${providerId}/proxy`, { method: 'PUT', body: { proxy_id: proxyId } });
}
