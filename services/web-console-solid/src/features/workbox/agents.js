// The sentences of the agents card, kept apart from the JSX so they can be tested.
//
// The card itself only draws: which agent is busy and why a restart interrupts
// an answer is a rule about the runtime state, and a rule deserves a test.

const LABELS = {
  claude: 'Claude Code',
  codex: 'Codex',
  opencode: 'OpenCode',
};

// The order of the two sign-in modes, as the segmented control shows them.
export const MODE_OPTIONS = [
  { value: 'subscription', label: 'Subscription' },
  { value: 'gateway', label: 'Gateway' },
];

// Display name of an agent id.
export function agentLabel(agent) {
  return LABELS[agent] || agent;
}

// The word for a sign-in mode, capitalised for a sentence or a dialog title.
export function modeLabel(mode) {
  if (mode === 'subscription') return 'Subscription';
  if (mode === 'gateway') return 'Gateway';
  return mode;
}

// The sessions of an agent, never null.
function sessionsOf(state) {
  return (state && state.sessions) || [];
}

// A short name for one tmux session, falling back to its pane id.
function sessionName(session) {
  return session.tmux_session || session.pane_id || 'a session';
}

// Seconds of idling as words: "now", "42 s", "3 min".
function idleText(seconds) {
  if (typeof seconds !== 'number' || seconds <= 0) return 'now';
  if (seconds < 60) return `${Math.round(seconds)} s`;
  return `${Math.floor(seconds / 60)} min`;
}

// "t-ab12, t-cd34" for a list of sessions, or an empty string.
function nameList(sessions) {
  const names = sessions.map(sessionName).filter(Boolean);
  return names.length ? ` (${names.join(', ')})` : '';
}

// The sentence the switch dialog shows, or null when nothing runs.
//
// Working sessions come first because a restart would cut their answer off; a
// server agent that is busy without a pane is named in the singular. Idle
// sessions still get a sentence, because the dialog needs to say that switching
// restarts them.
export function busyMessage(state) {
  const sessions = sessionsOf(state);
  const busy = sessions.filter((session) => session.busy);
  if (busy.length > 0) {
    const verb = busy.length === 1 ? 'session is' : 'sessions are';
    const pronoun = busy.length === 1 ? 'its' : 'their';
    return `${busy.length} ${verb} working right now${nameList(busy)}. Restarting stops ${pronoun} current answer.`;
  }
  if (state && state.busy) {
    return 'The agent is working right now. Restarting stops its current answer.';
  }
  if (sessions.length > 0) {
    const verb = sessions.length === 1 ? 'session is' : 'sessions are';
    return `${sessions.length} ${verb} open and idle${nameList(sessions)}. Restarting now does not interrupt an answer.`;
  }
  return null;
}

// The open sessions of an agent in one line: "2 sessions: t-ab12 (idle for 45 s), …".
export function sessionSummary(state) {
  const sessions = sessionsOf(state);
  if (sessions.length === 0) return null;
  const parts = sessions.map((session) => {
    const how = session.busy ? 'working' : `idle for ${idleText(session.idle_seconds)}`;
    return `${sessionName(session)} (${how})`;
  });
  const word = sessions.length === 1 ? 'session' : 'sessions';
  return `${sessions.length} ${word}: ${parts.join(', ')}`;
}

// The login line: "Signed in · me@x" / "Not signed in" / "Login unknown".
export function loginLine(state) {
  if (!state || state.signed_in === null || state.signed_in === undefined) return 'Login unknown';
  if (state.signed_in === false) return 'Not signed in';
  return state.account ? `Signed in · ${state.account}` : 'Signed in';
}

// The one-time hint a subscription needs when no login is stored yet, or null.
export function loginHint(state) {
  if (!state || state.mode !== 'subscription' || state.signed_in !== false) return null;
  const own = `Open a terminal and run \`${state.agent} login\` once; the login is kept.`;
  if (state.agent === 'claude') {
    return `Open a terminal and run \`claude login\` (or \`/login\` inside Claude Code) once; the login is kept.`;
  }
  return own;
}
