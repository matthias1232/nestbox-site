// Pure logic of the add-node wizard: which steps exist, what is valid, the call it makes.
// No JSX and no Solid, so `node --test` can run it.

// The longest node name OpenShell accepts, and the shape of a valid one.
export const NAME_LIMIT = 42;
const NAME_PATTERN = /^[a-z0-9]([a-z0-9-]*[a-z0-9])?$/;

// The three ways to get a machine, with the sentence shown on its option card.
export const SOURCES = [
  { id: 'script', title: 'Run a script', text: 'You get one line to paste on any Linux machine as root. The node dials in by itself.' },
  { id: 'connect', title: 'Connect by IP', text: 'This control plane logs in over SSH once and installs the node. The login is never stored.' },
  { id: 'rent', title: 'Rent a machine', text: 'Rent a server from a provider account for a while. It is handed back when its time is up.' },
];

// The progress steps the quick-deploy job writes into its log (contract C8).
export const PROGRESS_STEPS = [
  { id: 'provision', label: 'Provision' },
  { id: 'join', label: 'Join' },
  { id: 'probe', label: 'Probe' },
  { id: 'install_model', label: 'Install model' },
  { id: 'done', label: 'Done' },
];

// A blank wizard state; `siteId` is filled once the sites are loaded.
export function blankState() {
  return {
    source: 'script',
    siteId: '',
    name: '',
    connect: { address: '', sshUser: 'root', sshPort: '22', credential: 'password', password: '', privateKey: '', wgPort: '51820', lock: 'strict' },
    rent: { accountId: '', hostType: '', location: '', ttlMinutes: '120', idleMinutes: '30' },
    modelKey: '',
  };
}

// The wizard steps for one source: connect and rent have a details step, script has none.
export function steps(source) {
  const list = [
    { id: 'source', label: 'Source' },
    { id: 'placement', label: 'Placement' },
  ];
  if (source === 'connect') list.push({ id: 'details', label: 'Connection' });
  if (source === 'rent') list.push({ id: 'details', label: 'Machine' });
  list.push({ id: 'model', label: 'Model' }, { id: 'review', label: 'Review' });
  return list;
}

// An error text for a node name, or null when it is fine.
export function nameProblem(name) {
  const value = String(name || '').trim();
  if (!value) return 'Enter a node name.';
  if (value.length > NAME_LIMIT) return `Use at most ${NAME_LIMIT} characters.`;
  if (!NAME_PATTERN.test(value)) return 'Use lowercase letters, digits and dashes; start and end with a letter or digit.';
  return null;
}

function positiveInt(text, min, max) {
  const n = Number(text);
  return Number.isInteger(n) && n >= min && n <= max;
}

// Why a step cannot be left yet (a sentence), or null when it can.
export function validate(step, state) {
  if (step === 'placement') {
    if (!state.siteId) return 'Choose a site.';
    return nameProblem(state.name);
  }
  if (step === 'details' && state.source === 'connect') {
    const c = state.connect;
    if (!c.address.trim()) return 'Enter the machine address.';
    if (!c.sshUser.trim()) return 'Enter the SSH user.';
    if (!positiveInt(c.sshPort, 1, 65535)) return 'The SSH port must be 1 to 65535.';
    if (!positiveInt(c.wgPort, 1, 65535)) return 'The WireGuard port must be 1 to 65535.';
    if (c.credential === 'password' ? !c.password : !c.privateKey.trim()) return 'Enter the password or private key.';
    return null;
  }
  if (step === 'details' && state.source === 'rent') {
    const r = state.rent;
    if (!r.accountId) return 'Choose a provider account.';
    if (!r.hostType) return 'Choose a machine type.';
    if (!positiveInt(r.ttlMinutes, 15, 10080)) return 'The lifetime must be 15 to 10080 minutes.';
    if (!positiveInt(r.idleMinutes, 0, 10080)) return 'The idle limit must be 0 to 10080 minutes.';
    return null;
  }
  return null;
}

// The first problem of any step, as `{ step, message }`, or null when the whole wizard is valid.
export function firstProblem(state) {
  for (const step of steps(state.source)) {
    const message = validate(step.id, state);
    if (message) return { step: step.id, message };
  }
  return null;
}

// The request body for `POST /nodes/quick` (contract C8). With `mask`, secrets read "********".
// `ssh_port` and `wg_port` are additions to C8 so the existing connect fields are kept.
export function bodyFor(state, mask = false) {
  let source;
  if (state.source === 'connect') {
    const c = state.connect;
    const usePassword = c.credential === 'password';
    source = {
      kind: 'connect',
      address: c.address.trim(),
      ssh_user: c.sshUser.trim(),
      ssh_port: Number(c.sshPort),
      wg_port: Number(c.wgPort),
      egress_lock: c.lock,
    };
    if (usePassword) source.ssh_password = mask ? '********' : c.password;
    else source.ssh_private_key = mask ? '********' : c.privateKey;
  } else if (state.source === 'rent') {
    const r = state.rent;
    source = {
      kind: 'rent',
      account_id: r.accountId,
      host_type: r.hostType,
      ttl_minutes: Number(r.ttlMinutes),
      idle_minutes: Number(r.idleMinutes),
    };
    if (r.location) source.location = r.location;
  } else {
    source = { kind: 'script' };
  }
  return {
    site_id: state.siteId,
    name: state.name.trim(),
    source,
    model_key: state.modelKey || null,
    wait_minutes: 20,
  };
}

// Plain-language lines for the review step, in the order the job does them.
export function reviewLines(state, names = {}) {
  const lines = [];
  const where = names.site ? `site ${names.site}` : 'the chosen site';
  if (state.source === 'script') {
    lines.push(`Create node ${state.name.trim()} in ${where} and issue a one-time join command.`);
    lines.push('Wait for you to run the command as root on the machine.');
  } else if (state.source === 'connect') {
    const c = state.connect;
    lines.push(`Log in to ${c.sshUser.trim()}@${c.address.trim()}:${c.sshPort} over SSH once (${c.credential === 'password' ? 'password' : 'private key'}, never stored).`);
    lines.push(`Install node ${state.name.trim()} in ${where}; WireGuard on UDP ${c.wgPort}.`);
    lines.push(c.lock === 'strict' ? 'Egress lock: strict, all traffic goes through this gateway.' : 'Egress lock: exit locally.');
  } else {
    const r = state.rent;
    lines.push(`Rent a ${r.hostType}${r.location ? ` in ${r.location}` : ''} from ${names.account || 'the provider account'}.`);
    lines.push(`Hand it back after ${r.ttlMinutes} min${Number(r.idleMinutes) > 0 ? `, or after ${r.idleMinutes} min empty` : ''}.`);
    lines.push(`It joins ${where} as node ${state.name.trim()}.`);
  }
  lines.push('Probe the node for hardware once it is online.');
  lines.push(state.modelKey
    ? `Deploy model ${names.model || state.modelKey} when the node is online (fit is checked after the probe).`
    : 'No model is deployed.');
  return lines;
}

// The state of each progress step from the job and its log messages.
// A step is `done` when a later step appeared or the job finished, `active` while it is the
// latest one seen, `failed` on the active step of a failed job, else `waiting`.
export function progressStates(job, logs, withModel) {
  const ids = PROGRESS_STEPS.map((s) => s.id).filter((id) => withModel || id !== 'install_model');
  let latest = -1;
  for (const row of logs || []) {
    const text = String(row.message || '').toLowerCase();
    ids.forEach((id, i) => {
      if (i > latest && (text.includes(id) || text.includes(id.replace('_', ' ')))) latest = i;
    });
  }
  const status = job?.status;
  const finished = status === 'completed' || status === 'succeeded';
  const failed = status === 'failed';
  return ids.map((id, i) => {
    let state = 'waiting';
    if (finished || i < latest) state = 'done';
    else if (i === latest) state = failed ? 'failed' : id === 'done' ? 'done' : 'active';
    else if (i === latest + 1 && latest < 0 && !failed && i === 0) state = 'active';
    return { id, label: PROGRESS_STEPS.find((s) => s.id === id).label, state };
  });
}

// Whether a job status means no more progress will come.
export function jobFinished(job) {
  return ['completed', 'succeeded', 'failed', 'cancelled'].includes(job?.status);
}

// A models list reduced to the ones with an artifact (those can be deployed to a node).
export function deployableModels(items) {
  return (items || []).filter((m) => m.artifact_ref).sort((a, b) => String(a.name || a.key).localeCompare(String(b.name || b.key)));
}
