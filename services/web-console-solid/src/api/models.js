// Model catalog, hardware, and the facets behind the filter chips.

import { request, query } from './client.js';

/**
 * The catalog, classified against this machine.
 *
 * `refresh` forces a fresh probe. The filters are passed to the server so a
 * caller can ask for "only what fits" without loading the rest; the console
 * applies its live filters to the verdicts the server returned.
 */
export function catalog(refresh = false, filters = {}, node) {
  return request(`models${query({
    refresh_hardware: refresh ? 'true' : undefined,
    node,
    family: filters.family,
    publisher: filters.publisher,
    runtime: filters.runtime,
    cpu_only: filters.cpuOnly ? 'true' : undefined,
    unified_only: filters.unifiedOnly ? 'true' : undefined,
    fits: filters.fits === undefined ? undefined : String(filters.fits),
  })}`);
}

/**
 * Every install in the environment, one row per model per node.
 *
 * The row carries `model_key`, `model_name`, `node_id`, and `node_name`, so a
 * screen that only holds an install id can name the model and the machine
 * without a second call.
 */
export function installs(node) {
  return request(`models/installs${query({ node })}`);
}

export function hardware(refresh = false) {
  return request(`models/hardware${query({ refresh_hardware: refresh ? 'true' : undefined })}`);
}

/** One model as a requirement profile, for a detail view. */
export function requirements(key) {
  return request(`models/requirements${query({ key })}`);
}

/** Counts per family, publisher, runtime, and verdict. */
export function facets() {
  return request('models/facets');
}

/**
 * Install on a node. The answer is the reservation it made, or
 * `422 node.insufficient_resources` naming what the node's other installs hold.
 */
export function install(key, node, force = false) {
  return request(`models/${key}/install`, { method: 'POST', body: { node, force } });
}

export function uninstall(key, node) {
  return request(`models/${key}/uninstall`, { method: 'POST', body: node ? { node } : {} });
}

/**
 * Refresh the catalogue from Hugging Face, Ollama, and NGC.
 *
 * Runs inside the request, so the counts come back with it. A curated row is
 * never overwritten; the per-source counts say what changed.
 */
export function sync() {
  return request('models/sync', { method: 'POST' });
}

/**
 * Edit an installed model: `{ node, enabled, reserved_ram_bytes, reserved_vram_bytes }`,
 * every field optional. Off stops the server and frees its memory; a new RAM
 * figure on a running model restarts it. `422 node.insufficient_resources`
 * says what the node has left.
 */
export function updateInstall(key, change) {
  return request(`models/${key}/install`, { method: 'PATCH', body: change });
}

/** The last log lines a served model wrote. */
export function logs(key, node, tail = 200) {
  return request(`models/${key}/logs${query({ node, tail })}`);
}

/**
 * Sample the served model now and return its stored series.
 *
 * The answer carries `available`, the `current` sample, the `series`, and the
 * `consumers` (sandboxes whose provider profile points at this model).
 */
export function metrics(key, node, limit = 120) {
  return request(`models/${key}/metrics${query({ node, limit })}`);
}

/**
 * Install one model on many nodes as a job (contract C6).
 *
 * `body`: { nodes, site_id, environment_wide, force, make_cluster: { name, strategy } }.
 * The answer is `{ job_id, targets }`.
 */
export function installBulk(key, body) {
  return request(`models/${encodeURIComponent(key)}/install-bulk`, { method: 'POST', body });
}
