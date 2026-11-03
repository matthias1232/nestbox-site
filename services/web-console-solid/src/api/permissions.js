// Who has which role in which scope (contract C13).

import { request, query } from './client.js';

// `scopeType`: organization | environment | sandbox.
export function list(scopeType, scopeId) {
  return request(`permissions${query({ scope_type: scopeType, scope_id: scopeId })}`);
}

// Give a user a role: `{ user_id, scope_type, scope_id, role }`.
export function set(body) {
  return request('permissions', { method: 'PUT', body });
}

// Take a role away: `{ user_id, scope_type, scope_id }`.
export function remove(body) {
  return request('permissions', { method: 'DELETE', body });
}
