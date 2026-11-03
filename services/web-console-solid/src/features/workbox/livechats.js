// Turn recorded model calls into conversations for the live chat panel.
//
// Reading a body already lives in `features/sandboxes/chats.js`; this file only
// turns one traffic row plus its body into a turn, groups turns by session, and
// shortens text for display. All of it is plain logic, so it is tested without a
// DOM (see `livechats.test.js`).

import { readRequest, readResponse, sessionOf, titleOf } from '../sandboxes/chats.js';

// How many bodies one refresh may fetch. The rest waits for the next refresh,
// so a sandbox with a long history does not fire a hundred requests at once.
export const PER_REFRESH = 6;

// Collapse all whitespace to single spaces and trim, so a model answer that
// arrives pre-wrapped does not take ten lines of screen height.
export function flatten(text) {
  return String(text ?? '').replace(/\s+/g, ' ').trim();
}

// `text` as one line of at most `n` characters, cut with an ellipsis when it is
// longer. Used for the prompt and the answer of a turn before "show more".
export function clip(text, n) {
  const flat = flatten(text);
  if (flat.length <= n) return flat;
  return `${flat.slice(0, n).trimEnd()}…`;
}

// True when `clip(text, n)` would shorten the text, which is when the panel
// offers "show more".
export function hasMore(text, n) {
  return flatten(text).length > n;
}

// One turn out of a traffic row and its recorded body.
//
// A body that cannot be read becomes a turn marked `unreadable`, so the panel
// can say that instead of silently dropping the call. The same shape is used by
// the chats tab of the sandbox deck.
export function turnOf(item, body) {
  if (!body) {
    return {
      id: item.id,
      ts: item.ts,
      host: item.host,
      path: item.path,
      status: item.status,
      truncated: false,
      ask: { prompt: '' },
      answer: { text: '' },
      session: null,
      title: null,
      unreadable: true,
    };
  }
  const ask = readRequest(body.request_body);
  const answer = readResponse(body.response_body);
  return {
    id: item.id,
    ts: item.ts,
    host: item.host,
    path: item.path,
    status: item.status,
    truncated: body.truncated === true,
    ask,
    answer,
    session: sessionOf(body.request_body),
    title: titleOf(answer.text),
  };
}

// Group turns into conversations, newest conversation first.
//
// Calls that belong together (one agent session) become one group. A call whose
// answer is a title the client asked the model to write is that group's title
// and not a turn of its own; a call with neither a question nor an answer is
// bookkeeping of the client and is dropped. Inside a group the turns are newest
// first, so `turns.slice(0, 3)` is "the last three".
//
// Each group is `{session, title, model, last_ts, turns}`.
export function groupTurns(turns) {
  const bySession = new Map();
  for (const turn of turns || []) {
    const session = turn.session || 'none';
    if (!bySession.has(session)) bySession.set(session, { session, title: null, turns: [] });
    const group = bySession.get(session);
    if (turn.title) {
      // A title call says nothing about what was asked; it names the group.
      if (!group.title) group.title = turn.title;
      continue;
    }
    if (!turn.unreadable && !turn.ask?.prompt && !turn.answer?.text && !turn.answer?.error) continue;
    group.turns.push(turn);
  }

  return [...bySession.values()]
    .filter((group) => group.turns.length > 0)
    .map((group) => {
      const ordered = [...group.turns].sort((a, b) => String(b.ts).localeCompare(String(a.ts)));
      return {
        session: group.session,
        title: group.title,
        model: ordered[0].ask?.model || null,
        last_ts: ordered[0].ts,
        turns: ordered,
      };
    })
    .sort((a, b) => String(b.last_ts).localeCompare(String(a.last_ts)));
}
