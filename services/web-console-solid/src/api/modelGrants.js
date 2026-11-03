// Which models a sandbox may use, granted to the environment, a user or it.
import { request, query } from './client.js';

// The grants the caller may see.
export function list(params) {
  return request(`model-grants${query(params)}`);
}

// Grant a model.
export function create(body) {
  return request('model-grants', { method: 'POST', body });
}

// Revoke a grant.
export function remove(id) {
  return request(`model-grants/${id}`, { method: 'DELETE' });
}

// The models one sandbox sees after inheritance.
export function sandboxModels(sandboxId) {
  return request(`sandboxes/${sandboxId}/models`);
}

// Load a model and answer one tiny request, so the next real one is fast.
export function warm(key) {
  return request(`models/${encodeURIComponent(key)}/warm`, { method: 'POST', body: {} });
}
