// Model clusters: installs of one model behind one provider (contract C7).

import { request } from './client.js';

// Every cluster.
export function list() {
  return request('model-clusters');
}

// One cluster with its members.
export function get(id) {
  return request(`model-clusters/${id}`);
}

// Create a cluster: `{ name, model_key, strategy, install_ids }`.
export function create(body) {
  return request('model-clusters', { method: 'POST', body });
}

// Rename a cluster or change its strategy.
export function update(id, body) {
  return request(`model-clusters/${id}`, { method: 'PUT', body });
}

// Delete a cluster and its provider profile.
export function remove(id) {
  return request(`model-clusters/${id}`, { method: 'DELETE' });
}

// Add an install to a cluster.
export function addMember(id, installId) {
  return request(`model-clusters/${id}/members`, { method: 'POST', body: { install_id: installId } });
}

// Remove an install from a cluster.
export function removeMember(id, installId) {
  return request(`model-clusters/${id}/members/${installId}`, { method: 'DELETE' });
}
