// Plain-English names and summaries for jobs ("tasks"), shared by the Tasks page and the dock.

const LABELS = {
  sandbox_bulk_action: 'Bulk action on sandboxes',
  sandbox_create: 'Create sandbox',
  sandbox_delete: 'Delete sandbox',
  policy_rollout: 'Roll out policy',
  gateway_config_sync: 'Sync gateway configuration',
  model_install: 'Install model',
  model_update: 'Update model',
  model_uninstall: 'Remove model',
  model_bulk_install: 'Deploy model on nodes',
  node_drain: 'Drain node',
  node_quick_deploy: 'Add node',
  certificate_renew: 'Renew certificate',
  certificate_expiry_check: 'Check certificate expiry',
  provider_refresh: 'Refresh providers',
  policy_reconciliation: 'Reconcile policies',
  address_pool_reconcile: 'Reconcile address pool',
  model_update_check: 'Check for model updates',
  session_cleanup: 'Clean up sessions',
  catalog_sync: 'Sync model catalog',
  recipe_build: 'Build image',
  metrics_rollup: 'Roll up metrics',
  metrics_partition_maintenance: 'Maintain metrics partitions',
  telemetry_partition_maintenance: 'Maintain telemetry partitions',
  egress_partition_maintenance: 'Maintain egress partitions',
};

// A readable label for a job kind; an unknown kind becomes "Some kind" from its snake_case name.
export function labelFor(kind) {
  if (LABELS[kind]) return LABELS[kind];
  const words = String(kind || 'task').replace(/_/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

// The badge tone (ok, warn, bad, info, muted) for a job status.
export function toneFor(status) {
  switch (status) {
    case 'succeeded': return 'ok';
    case 'running': return 'info';
    case 'pending':
    case 'partial': return 'warn';
    case 'failed': return 'bad';
    default: return 'muted';
  }
}

// One short line of progress: "12 of 20 done, 1 failed".
export function summaryLine(job) {
  const total = Number(job?.total) || 0;
  const done = Number(job?.completed) || 0;
  const failed = Number(job?.failed) || 0;
  if (total === 0) return job?.status === 'running' ? 'Running' : 'No targets';
  const base = `${done} of ${total} done`;
  return failed > 0 ? `${base}, ${failed} failed` : base;
}

// Whether the kind fans out to targets and so has a per-target result table.
export function hasTargets(kind) {
  return ['sandbox_bulk_action', 'model_bulk_install', 'node_drain'].includes(kind);
}

// Per-target results from log rows. A row's message is either JSON `{sandbox_id|node, ok, message}`
// or plain text; only rows that carry a target and an outcome become results.
export function parseTargets(rows) {
  const out = [];
  for (const row of rows || []) {
    const text = String(row.message || '');
    if (!text.startsWith('{')) continue;
    try {
      const value = JSON.parse(text);
      const target = value.sandbox_id || value.node || value.target;
      if (target && typeof value.ok === 'boolean') {
        out.push({ target: String(target), ok: value.ok, message: String(value.message || ''), kind: value.node ? 'node' : 'sandbox' });
      }
    } catch {
      // Not a result line.
    }
  }
  return out;
}

// The ids of failed targets that a retry can use, or [] when there are none.
export function failedTargets(rows) {
  return parseTargets(rows).filter((t) => !t.ok).map((t) => t.target);
}

// Log rows as the plain text LogView reads: `<time> [level] message`.
export function logText(rows) {
  return (rows || []).map((r) => `${r.ts} [${r.level}] ${r.message}`).join('\n');
}

// How long a job ran, as "2 min 05 s", or "" when it has not started.
export function durationText(job, now = Date.now()) {
  if (!job?.started_at) return '';
  const end = job.finished_at ? Date.parse(job.finished_at) : now;
  const seconds = Math.max(0, Math.round((end - Date.parse(job.started_at)) / 1000));
  if (seconds < 60) return `${seconds} s`;
  const minutes = Math.floor(seconds / 60);
  const rest = String(seconds % 60).padStart(2, '0');
  if (minutes < 60) return `${minutes} min ${rest} s`;
  return `${Math.floor(minutes / 60)} h ${String(minutes % 60).padStart(2, '0')} min`;
}
