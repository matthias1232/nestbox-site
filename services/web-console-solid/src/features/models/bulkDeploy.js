// Pure logic behind the bulk deploy wizard: which nodes can take a model, how a selection is
// built, the request body (contract C6) and the per-node progress shown after Finish.
//
// The fit computed here is a PREVIEW so the operator sees likely skips before pressing Finish.
// The server decides: `targets[].fit` / `skipped` in the answer of `installBulk` always win.

const GIB = 1073741824;

// What a model needs, in bytes, with the "recommended" figure falling back to the minimum.
// `needsGpu` is true only for a GPU-only runtime; hybrid models also run on a CPU node.
export function modelNeeds(model) {
  const m = model || {};
  return {
    needsGpu: m.runtime === 'cuda' && !m.is_cpu_only,
    minVram: Number(m.min_vram_bytes || 0),
    recVram: Number(m.recommended_vram_bytes || m.min_vram_bytes || 0),
    minRam: Number(m.min_ram_bytes || 0),
    recRam: Number(m.recommended_ram_bytes || m.min_ram_bytes || 0),
  };
}

// A node's free memory in the pool the model would use: VRAM on a GPU node, RAM otherwise.
function pool(needs, node) {
  const budget = node.budget || {};
  const gpu = Boolean(node.gpu) && Number(budget.vram_total_bytes || 0) > 0;
  if (gpu && (needs.needsGpu || needs.minVram > 0)) {
    return { free: Number(budget.vram_free_bytes || 0), min: needs.minVram, rec: needs.recVram };
  }
  return { free: Number(budget.ram_free_bytes || 0), min: needs.minRam, rec: needs.recRam };
}

// Preview verdict of one model on one node: { fit: 'fits' | 'tight' | 'no', reason }.
// `no` covers offline, no GPU, and below the minimum; `tight` is minimum met, recommended not.
export function nodeFit(model, node) {
  if (!node || node.online === false) return { fit: 'no', reason: 'The node is offline.' };
  const needs = modelNeeds(model);
  if (needs.needsGpu && !node.gpu) return { fit: 'no', reason: 'The model needs a GPU and this node has none.' };
  const { free, min, rec } = pool(needs, node);
  if (free < min) {
    return { fit: 'no', reason: `Needs ${(min / GIB).toFixed(1)} GiB, ${(free / GIB).toFixed(1)} GiB free.` };
  }
  if (free < rec) {
    return { fit: 'tight', reason: `Meets the minimum; ${(rec / GIB).toFixed(1)} GiB recommended.` };
  }
  return { fit: 'fits', reason: 'Fits with the recommended memory.' };
}

// Every node of the tree with its place: [{ node, dcId, dcName, siteId, siteName }].
export function treeNodes(tree) {
  const out = [];
  const walk = (item, place) => {
    if (!item) return;
    const here = { ...place };
    if (item.kind === 'data_center') { here.dcId = item.id; here.dcName = item.name; }
    if (item.kind === 'site') { here.siteId = item.id; here.siteName = item.name; }
    if (item.kind === 'node') out.push({ node: item, ...here });
    (item.children || []).forEach((child) => walk(child, here));
  };
  walk(tree, { dcId: null, dcName: 'Data center', siteId: null, siteName: 'Site' });
  return out;
}

// Candidate rows for the picker. `nodes` is `api/nodes.js list()` (budget, container_gpu),
// `tree` the inventory root (place, tunnel state). `filters`: { model, text, onlyFitting, dc, site }.
// A node missing from `nodes` still appears from the tree alone, with the tree's own budget.
export function candidateNodes(nodes, tree, filters = {}) {
  const byId = new Map((nodes || []).map((n) => [n.id, n]));
  const placed = treeNodes(tree);
  const seen = new Set();
  const rows = [];
  const push = (id, name, status, gpu, budget, place) => {
    seen.add(id);
    const online = status === 'online' || status === 'degraded';
    const row = { id, name, status, online, gpu: Boolean(gpu), budget: budget || {}, ...place };
    const verdict = nodeFit(filters.model, row);
    rows.push({ ...row, fit: verdict.fit, reason: verdict.reason });
  };
  for (const entry of placed) {
    const api = byId.get(entry.node.id);
    push(
      entry.node.id,
      entry.node.name,
      api?.status || entry.node.status,
      api ? api.container_gpu : entry.node.gpu,
      api?.budget || entry.node.budget,
      { dcId: entry.dcId, dcName: entry.dcName, siteId: entry.siteId, siteName: entry.siteName },
    );
  }
  for (const api of nodes || []) {
    if (seen.has(api.id)) continue;
    push(api.id, api.name, api.status, api.container_gpu, api.budget,
      { dcId: null, dcName: 'Data center', siteId: null, siteName: 'Site' });
  }
  const needle = String(filters.text || '').trim().toLowerCase();
  return rows.filter((r) => {
    if (filters.onlyFitting && r.fit === 'no') return false;
    if (filters.dc && r.dcId !== filters.dc) return false;
    if (filters.site && r.siteId !== filters.site) return false;
    return !needle || `${r.name} ${r.siteName} ${r.dcName}`.toLowerCase().includes(needle);
  }).sort((a, b) => `${a.dcName}/${a.siteName}/${a.name}`.localeCompare(`${b.dcName}/${b.siteName}/${b.name}`));
}

// Candidates grouped data center › site: [{ dcId, dcName, sites: [{ siteId, siteName, nodes }] }].
export function groupNodes(cands) {
  const dcs = new Map();
  for (const c of cands) {
    const dcKey = c.dcId || 'none';
    if (!dcs.has(dcKey)) dcs.set(dcKey, { dcId: c.dcId, dcName: c.dcName, sites: new Map() });
    const dc = dcs.get(dcKey);
    const siteKey = c.siteId || 'none';
    if (!dc.sites.has(siteKey)) dc.sites.set(siteKey, { siteId: c.siteId, siteName: c.siteName, nodes: [] });
    dc.sites.get(siteKey).nodes.push(c);
  }
  return [...dcs.values()].map((dc) => ({ ...dc, sites: [...dc.sites.values()] }));
}

// Ids of every candidate that fits or is tight ("All that fit").
export function selectFitting(cands) {
  return new Set(cands.filter((c) => c.fit !== 'no').map((c) => c.id));
}

// Ids of the candidates of one site that fit or are tight ("Whole site").
export function selectBySite(cands, siteId) {
  return new Set(cands.filter((c) => c.siteId === siteId && c.fit !== 'no').map((c) => c.id));
}

// Ids of the candidates of one data center that fit or are tight ("Whole data center").
export function selectByDc(cands, dcId) {
  return new Set(cands.filter((c) => c.dcId === dcId && c.fit !== 'no').map((c) => c.id));
}

// The wizard's counters for the current selection.
export function counts(cands, selected) {
  const chosen = cands.filter((c) => selected.has(c.id));
  return {
    selected: chosen.length,
    fits: chosen.filter((c) => c.fit === 'fits').length,
    tight: chosen.filter((c) => c.fit === 'tight').length,
    skipped: chosen.filter((c) => c.fit === 'no').length,
  };
}

// The count line "12 selected · 2 tight · 3 skipped" (zero parts are left out).
export function summary(cands, selected) {
  const c = counts(cands, selected);
  const parts = [`${c.selected} selected`];
  if (c.tight) parts.push(`${c.tight} tight`);
  if (c.skipped) parts.push(`${c.skipped} skipped`);
  return parts.join(' · ');
}

// The request body of `installBulk` (contract C6). Node names are sent; the server resolves them.
// `state`: { selected: Set(id), cands, force, makeCluster, clusterName, strategy }.
export function bodyFor(state) {
  const names = state.cands.filter((c) => state.selected.has(c.id)).map((c) => c.name);
  const body = { nodes: names, site_id: null, environment_wide: false, force: Boolean(state.force) };
  const name = String(state.clusterName || '').trim();
  if (state.makeCluster && name) {
    body.make_cluster = { name, strategy: state.strategy || 'round_robin' };
  }
  return body;
}

// Why the Nodes step cannot continue yet, or null.
export function nodesProblem(cands, selected) {
  const c = counts(cands, selected);
  if (c.selected === 0) return 'Select at least one node.';
  if (c.selected === c.skipped) return 'Every selected node is skipped; pick a node that fits.';
  return null;
}

// Why the Options step cannot continue yet, or null.
export function optionsProblem(state) {
  if (state.makeCluster && !String(state.clusterName || '').trim()) return 'Name the cluster.';
  return null;
}

// Per-node progress rows. `targets` is the server answer ([{ node, fit, skipped }]), `installs` the
// installs of this model (`api/models.js installs()`), `logs` the job's log rows. Each row:
// { node, state: 'queued' | 'installing' | 'installed' | 'failed' | 'skipped', percent, tail }.
export function progressRows(targets, installs, logs) {
  const lines = (logs || []).map((r) => String(r.message || ''));
  return (targets || []).map((t) => {
    if (t.skipped) return { node: t.node, state: 'skipped', percent: 0, tail: t.skipped };
    const install = (installs || []).find((i) => i.node_name === t.node || i.node_id === t.node);
    const mention = [...lines].reverse().find((l) => l.includes(t.node)) || '';
    let state = 'queued';
    let percent = 0;
    if (install) {
      if (install.state === 'installed') { state = 'installed'; percent = 100; }
      else if (install.state === 'failed') { state = 'failed'; percent = 100; }
      else if (install.state === 'starting') { state = 'installing'; percent = 80; }
      else if (install.state === 'downloading') { state = 'installing'; percent = 50; }
      else { state = 'installing'; percent = 20; }
    }
    return { node: t.node, state, percent, tail: install?.error || mention };
  });
}

// Overall progress of those rows: { done, failed, total, percent, finished }. Skipped rows do not count.
export function overall(rows) {
  const live = rows.filter((r) => r.state !== 'skipped');
  const done = live.filter((r) => r.state === 'installed').length;
  const failed = live.filter((r) => r.state === 'failed').length;
  const total = live.length;
  return { done, failed, total, percent: total ? Math.round(((done + failed) / total) * 100) : 100, finished: done + failed >= total };
}

// The cluster id a finished job produced, or null.
export function clusterIdOf(job) {
  return job?.result?.cluster_id || null;
}

// Review lines: every call the wizard will make, in order, as plain sentences.
export function reviewLines(state, modelName) {
  const names = state.cands.filter((c) => state.selected.has(c.id)).map((c) => c.name);
  const lines = [`POST models/${state.modelKey}/install-bulk — install ${modelName || state.modelKey} on ${names.length} node(s): ${names.join(', ')}.`];
  if (state.force) lines.push('Nodes that do not fit are installed anyway (force).');
  const body = bodyFor(state);
  if (body.make_cluster) lines.push(`Afterwards the job creates cluster "${body.make_cluster.name}" (${body.make_cluster.strategy}) from every successful install.`);
  if (state.grant) lines.push('After success: POST model-grants — grant this model to the whole data center.');
  return lines;
}
