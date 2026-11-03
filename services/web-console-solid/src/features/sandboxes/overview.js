// Pure helpers for the two live columns of the sandbox list.
//
// The list itself is a paged inventory; the network mode, the "all allowed"
// badge and the sign-in modes come from two other endpoints (control overview
// and login sessions). Turning those answers into column text lives here, so it
// is testable without a DOM.

/** The short label of an egress mode, one or two words that fit a table cell. */
const MODE_LABELS = {
  transparent: 'Open all',
  hybrid: 'Chats',
  forward_proxy: 'Tunnel',
  off_allow_all: 'OFF · all allowed',
  direct: 'Direct',
};

/** How an egress mode is spoken: `transparent` reads as "Open all". */
export function modeLabel(mode) {
  if (!mode) return '—';
  return MODE_LABELS[mode] || mode;
}

/** Agent names as a column writes them. */
const AGENT_NAMES = { claude: 'Claude', codex: 'Codex', opencode: 'Opencode' };

/** The sign-in mode of every agent, as "Claude: subscription · Codex: gateway". */
export function agentModes(agents) {
  if (!Array.isArray(agents) || agents.length === 0) return '';
  return agents
    .map((agent) => `${AGENT_NAMES[agent.agent] || agent.agent}: ${agent.mode}`)
    .join(' · ');
}

/** The control overview keyed by sandbox id, so a row looks up its own entry. */
export function indexById(list) {
  const byId = {};
  for (const entry of list || []) {
    if (entry && entry.sandbox_id) byId[entry.sandbox_id] = entry;
  }
  return byId;
}
