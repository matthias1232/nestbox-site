// Provider accounts and rented machines (ADR 0050).

import { request } from './client.js';

export const accounts = () => request('host-accounts');
export const addAccount = (body) => request('host-accounts', { method: 'POST', body });
export const removeAccount = (id) => request(`host-accounts/${id}`, { method: 'DELETE' });
export const types = (id) => request(`host-accounts/${id}/types`);
export const hosts = () => request('hosts');
export const burst = (body) => request('hosts', { method: 'POST', body });
export const destroy = (id) => request(`hosts/${id}`, { method: 'DELETE' });
