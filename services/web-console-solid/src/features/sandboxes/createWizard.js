// The pure rules of the four-step "New sandbox" wizard (package F24).
//
// No DOM and no API calls: the component feeds in the wizard state and the node
// list, and sends what `bodyFor` and `followUps` describe. The server stays the
// authority; the hints below only keep an obvious mistake from reaching it.

const MIB = 1024 * 1024;

// Where the sandbox comes from. `pack` and `image` create through the sandbox API, `recipe` through
// the recipe's instantiate call.
export const SOURCES = ['pack', 'recipe', 'image'];

// The egress modes a new sandbox may start in, least open first.
export const EGRESS_MODES = [
  { value: 'forward_proxy', label: 'Tunnel only', help: 'Nothing is opened. Host, port and size are recorded; rules apply.' },
  { value: 'hybrid', label: 'Only agent chats', help: 'Model and agent connections are opened so chats are readable.' },
  { value: 'transparent', label: 'Open everything', help: 'Every TLS connection is opened and recorded.' },
];

// The agent packs that can sign in with the user's own account.
const OWN_LOGIN_PACKS = ['claude-code', 'opencode', 'openchamber'];

// A fresh wizard state with the API's own defaults.
export function blankState() {
  return {
    source: 'pack',
    name: '',
    pack_key: '',
    recipe_id: '',
    image: '',
    image_digest: '',
    login: 'gateway',
    node: '',
    memory_gib: '2',
    cpu: '1',
    pids: '512',
    gpu_count: '0',
    models: [],
    egress_mode: 'forward_proxy',
    policy_id: '',
    mounts: [],
  };
}

// The steps a source needs. A recipe fixes its own node and limits, so "Where" is left out.
export function stepsFor(kind) {
  const what = { id: 'what', label: 'What' };
  const where = { id: 'where', label: 'Where' };
  const access = { id: 'access', label: 'Access' };
  const review = { id: 'review', label: 'Review' };
  return kind === 'recipe' ? [what, access, review] : [what, where, access, review];
}

// Whether a pack offers the choice between the gateway key and the user's own account.
export function hasOwnLogin(packKey) {
  return OWN_LOGIN_PACKS.includes(packKey);
}

// A sha256 digest as the runtime writes it.
export function isDigest(value) {
  return /^sha256:[0-9a-f]{64}$/.test(String(value || '').trim());
}

const positive = (value) => {
  const n = Number(value);
  return Number.isFinite(n) && n > 0 ? n : null;
};

// Memory in bytes, or null when the field is not a positive number.
export function memoryBytes(state) {
  const gib = positive(state.memory_gib);
  return gib === null ? null : Math.round(gib * 1024) * MIB;
}

// The sentences that stop a step from going on; an empty list means the step is valid.
export function validate(step, state) {
  const found = [];
  if (step === 'what') {
    if (!String(state.name || '').trim()) found.push('A sandbox name is required.');
    if (state.source === 'pack' && !state.pack_key) found.push('Choose an agent pack.');
    if (state.source === 'recipe' && !state.recipe_id) found.push('Choose a recipe.');
    if (state.source === 'image') {
      if (!state.pack_key) found.push('Choose the agent pack that runs on the image.');
      if (!String(state.image || '').trim()) found.push('An image reference is required.');
      if (!isDigest(state.image_digest)) found.push('The image digest must look like sha256:<64 hex digits>; a tag alone is not accepted.');
    }
  }
  if (step === 'where') {
    if (memoryBytes(state) === null) found.push('Memory is a positive number of GiB.');
    if (positive(state.cpu) === null) found.push('CPU is a positive number of cores.');
    const pids = Number(state.pids);
    if (!Number.isInteger(pids) || pids <= 0) found.push('PIDs is a positive whole number.');
    const gpus = Number(state.gpu_count);
    if (!Number.isInteger(gpus) || gpus < 0) found.push('GPUs is zero or a positive whole number.');
  }
  if (step === 'access') {
    const paths = new Set();
    for (const mount of state.mounts || []) {
      const path = String(mount.mount_path || '').trim();
      if (!path.startsWith('/')) found.push('Every mount path must start with /.');
      else if (paths.has(path)) found.push(`The path ${path} is used twice.`);
      paths.add(path);
    }
  }
  return found;
}

// "fits", "tight", "no GPU", "not measured" or "offline" for one node, with a tone for the badge.
export function nodeHint(node, state) {
  if (!node) return { text: '', tone: 'muted' };
  if (node.status && node.status !== 'online' && node.status !== 'ready' && node.is_local !== true) {
    return { text: 'offline', tone: 'bad' };
  }
  const wantGpu = Number(state.gpu_count) || 0;
  const budget = node.budget || {};
  if (wantGpu > 0 && !(node.container_gpu || (budget.gpu_count || 0) >= wantGpu)) {
    return { text: 'no GPU', tone: 'bad' };
  }
  if (node.hardware_known !== true) return { text: 'not measured', tone: 'muted' };
  const want = memoryBytes(state);
  if (want === null) return { text: '', tone: 'muted' };
  const free = budget.ram_free_bytes || 0;
  if (want > free) return { text: 'too big', tone: 'bad' };
  const cores = Number(state.cpu) || 0;
  if (budget.cpu_cores > 0 && cores > budget.cpu_cores) return { text: 'too few cores', tone: 'bad' };
  // Under a quarter of the free memory left after the sandbox counts as tight.
  if (free - want < free * 0.25) return { text: 'tight', tone: 'warn' };
  return { text: 'fits', tone: 'ok' };
}

// The node "Let Nestbox pick" would choose: the fitting node with the most free memory, else null.
export function leastLoaded(nodes, state) {
  const fitting = (nodes || []).filter((node) => {
    const hint = nodeHint(node, state);
    return hint.tone === 'ok' || hint.tone === 'warn';
  });
  fitting.sort((a, b) => (b.budget?.ram_free_bytes || 0) - (a.budget?.ram_free_bytes || 0));
  return fitting[0] || null;
}

// The `POST /sandboxes` body for a pack or image source. A recipe uses `recipeCall` instead.
export function bodyFor(state) {
  const body = {
    name: String(state.name).trim(),
    pack_key: state.pack_key,
    memory_limit: `${Math.round(Number(state.memory_gib) * 1024)}Mi`,
    cpu_limit: Number(state.cpu),
    pids_limit: Number(state.pids),
    egress_mode: state.egress_mode,
  };
  if (state.source === 'image') {
    body.image = String(state.image).trim();
    body.image_digest = String(state.image_digest).trim();
  }
  if (Number(state.gpu_count) > 0) body.gpu_count = Number(state.gpu_count);
  if (state.node) body.node = state.node;
  if (state.login === 'own_account' && hasOwnLogin(state.pack_key)) body.login = 'own_account';
  return body;
}

// The create call itself, for the Review step: what is sent first.
export function createCall(state) {
  if (state.source === 'recipe') {
    return { label: 'Create from recipe', call: { api: 'recipes.instantiate', args: [state.recipe_id, String(state.name).trim()] } };
  }
  return { label: 'Create the sandbox', call: { api: 'sandboxes.create', args: [bodyFor(state)] } };
}

// The calls that follow a successful create, in the order they run. `models` maps a model name to the
// environment grant it copies, so a sandbox grant names the same provider profile.
export function followUps(state, sandboxId, grants = {}) {
  const calls = [];
  for (const name of state.models || []) {
    const grant = grants[name] || {};
    calls.push({
      label: `Grant model ${name}`,
      call: {
        api: 'modelGrants.create',
        args: [{
          scope: 'sandbox',
          sandbox_id: sandboxId,
          model_name: name,
          provider_profile_id: grant.provider_profile_id,
          ...(grant.upstream_model ? { upstream_model: grant.upstream_model } : {}),
        }],
      },
    });
  }
  if (state.policy_id) {
    calls.push({ label: 'Assign the policy', call: { api: 'policies.assignPolicy', args: [sandboxId, state.policy_id] } });
  }
  if ((state.mounts || []).length > 0) {
    calls.push({
      label: `Mount ${state.mounts.length} ${state.mounts.length === 1 ? 'root' : 'roots'}`,
      call: {
        api: 'mounts.put',
        args: [sandboxId, {
          roots: state.mounts.map((m) => ({ root_id: m.root_id, mount_path: m.mount_path.trim(), read_only: m.read_only === true })),
          apply: 'now',
        }],
      },
    });
  }
  return calls;
}

// The same calls as readable lines, for the Review step before a sandbox id exists.
export function previewLines(state) {
  const placeholder = '<new sandbox>';
  return [createCall(state), ...followUps(state, placeholder)].map((c) => c.label);
}
