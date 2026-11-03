// The logic behind the Models card, kept out of the component so it can be
// tested without a DOM: what a grant's scope reads as, what a readiness event
// means, and who may grant a model to one sandbox.
//
// Nothing here decides a verdict the server owns. The readiness state comes from
// the `models_changed` event and is only folded so the last answer per model
// wins; the card renders it, it never guesses it.

// The scope names the API returns for a grant, and how each one reads.
const SOURCE_LABELS = {
  sandbox: 'Sandbox',
  user: 'User',
  environment: 'Environment',
};

// Roles that manage models without being the owner of the sandbox. A viewer is
// never among them: read-only means read-only, even on a sandbox of their own.
const MANAGING_ROLES = new Set(['operator', 'admin']);

// Fold model readiness events into one entry per model; the last event wins.
//
// The events are the ones `onControl` hands out, so a control event may be
// mixed in; anything that is not a `models_changed` with a model name is
// ignored rather than shown as an unknown state.
export function readinessFromEvents(events) {
  const out = {};
  for (const event of events || []) {
    if (!event || event.type !== 'models_changed') continue;
    const detail = event.detail || {};
    if (!detail.model) continue;
    out[detail.model] = {
      state: detail.state || 'unknown',
      ms: Number.isFinite(detail.ms) ? detail.ms : null,
    };
  }
  return out;
}

// The badge for one grant's scope: sandbox, user or environment.
export function sourceLabel(source) {
  const key = String(source || '').trim().toLowerCase();
  if (SOURCE_LABELS[key]) return SOURCE_LABELS[key];
  if (!key) return 'Unknown';
  return key.charAt(0).toUpperCase() + key.slice(1);
}

// The badge of a model's readiness, or null when no event arrived yet.
//
// A model that never reported readiness shows no badge at all: "not checked"
// and "ready" must not look the same.
export function readinessBadge(entry) {
  if (!entry || !entry.state) return null;
  if (entry.state === 'warming') return { tone: 'warn', text: 'warming up' };
  if (entry.state === 'ready') {
    return { tone: 'ok', text: entry.ms === null || entry.ms === undefined ? 'ready' : `${entry.ms} ms` };
  }
  if (entry.state === 'failed') return { tone: 'bad', text: 'failed' };
  if (entry.state === 'missing') return { tone: 'bad', text: 'missing' };
  return { tone: 'muted', text: entry.state };
}

// Whether this person may add or revoke a model on one sandbox.
//
// Operators and admins may; everyone else only on a sandbox they own, and a
// viewer not even there. When the owner is unknown the answer is no, so the
// button is hidden instead of offered and then refused.
export function canManageModels(role, ownerUserId, userId) {
  if (MANAGING_ROLES.has(role)) return true;
  if (role === 'viewer' || !role) return false;
  if (!ownerUserId || !userId) return false;
  return String(ownerUserId) === String(userId);
}
