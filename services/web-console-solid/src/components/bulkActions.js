// Pure rules for the bulk bar: which actions a selection offers, whether the dialog input is
// complete, and the question the confirm dialog asks. No DOM, no API.

const SANDBOX_ACTIONS = [
  { id: 'start', label: 'Start', min: 'member' },
  { id: 'stop', label: 'Stop', min: 'member' },
  { id: 'restart', label: 'Restart', min: 'member' },
  { id: 'recreate', label: 'Recreate', min: 'operator', danger: true },
  { id: 'allow_all', label: 'Allow all 15 min', min: 'member', needs: null },
  { id: 'egress_mode', label: 'Egress mode…', min: 'operator', needs: 'mode' },
  { id: 'assign_policy', label: 'Assign policy…', min: 'operator', needs: 'policy_id' },
  { id: 'move', label: 'Move…', min: 'operator', needs: 'node' },
  { id: 'delete', label: 'Delete', min: 'admin', danger: true, needs: 'confirm' },
];

const NODE_ACTIONS = [
  { id: 'deploy_model', label: 'Deploy model…', min: 'operator', needs: 'model_key' },
  { id: 'probe_all', label: 'Probe all', min: 'operator' },
];

const RANK = { viewer: 0, member: 1, operator: 2, admin: 3 };

/** The four egress modes the select offers. */
export const EGRESS_MODES = ['forward_proxy', 'transparent', 'hybrid', 'off_allow_all'];

// Actions for a selection of `kind` ('sandbox' | 'node') for a role; unknown roles get none.
export function actionsFor(kind, role) {
  const rank = RANK[role];
  if (rank === undefined) return [];
  const all = kind === 'node' ? NODE_ACTIONS : kind === 'sandbox' ? SANDBOX_ACTIONS : [];
  return all
    .filter((action) => RANK[action.min] <= rank)
    .map(({ min, ...action }) => action);
}

// True when the dialog input the action needs is filled in correctly.
export function paramsValid(action, params = {}) {
  switch (action) {
    case 'egress_mode':
      return EGRESS_MODES.includes(params.mode);
    case 'assign_policy':
      return !!params.policy_id;
    case 'move':
      return !!params.node;
    case 'delete':
      return params.confirm === 'delete';
    case 'deploy_model':
      return !!(params.model_key && params.model_key.trim());
    default:
      return true;
  }
}

// The request body params for an action, with the fixed ones added (allow all lasts 15 minutes).
export function paramsFor(action, params = {}) {
  if (action === 'allow_all') return { minutes: 15 };
  if (action === 'delete') return { confirm: params.confirm };
  return { ...params };
}

// The question the dialog asks, such as "Stop 12 sandboxes?".
export function confirmText(action, count, kind = 'sandbox') {
  const noun = kind === 'node' ? 'node' : 'sandbox';
  const plural = noun === 'sandbox' ? 'sandboxes' : 'nodes';
  const things = `${count} ${count === 1 ? noun : plural}`;
  const verbs = {
    start: 'Start',
    stop: 'Stop',
    restart: 'Restart',
    recreate: 'Recreate',
    allow_all: 'Allow all traffic for 15 minutes on',
    egress_mode: 'Change the egress mode of',
    assign_policy: 'Assign a policy to',
    move: 'Move',
    delete: 'Delete',
    deploy_model: 'Deploy a model on',
    probe_all: 'Probe',
  };
  return `${verbs[action] || action} ${things}?`;
}

// True when the action must open the dialog first (it needs input or is dangerous).
export function needsDialog(action) {
  return !!(action.needs || action.danger || action.id === 'allow_all');
}
