// What the sandbox deck's tabs read: usage, processes, inventory, traffic, MCP,
// resources, and the host's load.

import { request, query } from './client.js';

export const metrics = (id, range = '1h') => request(`sandboxes/${id}/metrics${query({ range })}`);
export const processes = (id) => request(`sandboxes/${id}/processes`);
export const processAction = (id, pid, body) => request(`sandboxes/${id}/processes/${pid}/action`, { method: 'POST', body });
export const trafficBody = (id, eventId) => request(`sandboxes/${id}/traffic/${eventId}/body`);
export const liveView = (id, kind) => request(`sandboxes/${id}/live/${kind}`);
export const inventory = (id) => request(`sandboxes/${id}/inventory`);
export const refreshInventory = (id) => request(`sandboxes/${id}/inventory/refresh`, { method: 'POST' });
export const traffic = (id, params = {}) => request(`sandboxes/${id}/traffic${query(params)}`);

export const mcpRegistry = () => request('mcp/servers');
export const mcpAttached = (id) => request(`sandboxes/${id}/mcp/servers`);
export const mcpAttach = (id, serverKey) =>
  request(`sandboxes/${id}/mcp/servers`, { method: 'POST', body: { server_key: serverKey, enabled: true } });
export const mcpDetach = (id, key) => request(`sandboxes/${id}/mcp/servers/${key}`, { method: 'DELETE' });
export const mcpDiscover = (id, key) => request(`sandboxes/${id}/mcp/servers/${key}/discover`, { method: 'POST' });
export const mcpCalls = (id, limit = 30) => request(`sandboxes/${id}/mcp/calls${query({ limit, window: '24h' })}`);

export const setResources = (id, body) => request(`sandboxes/${id}/resources`, { method: 'PUT', body });
export const hostLoad = () => request('host/load');

export const recreate = (id) => request(`sandboxes/${id}/recreate`, { method: 'POST' });
export const reconfigureAgent = (id) => request(`sandboxes/${id}/agent-config`, { method: 'POST' });

// The sandbox's OpenShell supervisor: the supervisor row plus what the gateway reports.
export const supervisor = (id) => request(`sandboxes/${id}/supervisor`);

// How each running sandbox's agent is signed in (gateway key or own account), whether
// the supervisor is connected, and who has a session open.
export const loginSessions = () => request('sandboxes/login-sessions');
