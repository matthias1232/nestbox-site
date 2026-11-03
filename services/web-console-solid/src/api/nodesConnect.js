// Joining an existing machine by address, and how each node reaches the internet.
import { request } from './client.js';

// Install a machine as a node over SSH; answers 202 while the work runs.
export function connect(body) {
  return request('nodes/connect', { method: 'POST', body });
}

// The WireGuard tunnel of every node.
export function tunnels() {
  return request('nodes/tunnels');
}

// Choose how a node reaches the internet: strict or exit_local.
export function setEgressLock(nodeId, lock) {
  return request(`nodes/${nodeId}/egress-lock`, { method: 'PUT', body: { lock } });
}
