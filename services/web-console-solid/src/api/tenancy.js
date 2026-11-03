// Customers, data centers, sites, sandbox grants and invitations (ADR 0044, 0045, 0046).

import { request } from './client.js';

export const signInOptions = () => request('auth/options');
export const invitationPreview = (token) => request(`auth/invitations/${encodeURIComponent(token)}`);

export const organizations = () => request('organizations');
export const createCustomer = (name) => request('organizations', { method: 'POST', body: { name } });
export const createDataCenter = (name) => request('environments', { method: 'POST', body: { name } });

export const sites = () => request('sites');
export const createSite = (body) => request('sites', { method: 'POST', body });
export const removeSite = (id) => request(`sites/${id}`, { method: 'DELETE' });

export const grants = (sandboxId) => request(`sandboxes/${sandboxId}/grants`);
export const grant = (sandboxId, userId) =>
  request(`sandboxes/${sandboxId}/grants/${userId}`, { method: 'PUT' });
export const revoke = (sandboxId, userId) =>
  request(`sandboxes/${sandboxId}/grants/${userId}`, { method: 'DELETE' });

export const invitations = () => request('invitations');
export const invite = (body) => request('invitations', { method: 'POST', body });
export const withdraw = (id) => request(`invitations/${id}`, { method: 'DELETE' });
