// Split a masked text into the parts a reader has to see differently.
//
// A placeholder is the only trace of a replaced value that exists, so the split
// is what turns "the text says <IBAN_faf7e1c0>" into "here a value was replaced".
export const PLACEHOLDER = /<([A-Z_]+)_([0-9a-f]{8})>|\[REDACTED\]/g;

const KIND_BY_ACTION = {
  replaced: 'replaced',
  redacted: 'redacted',
  pseudonymized: 'pseudonymized',
  blocked: 'blocked',
};

/**
 * Split `text` at every placeholder and attach the incident that produced it.
 *
 * Returns `{parts, recorded}`: `parts` are the readable pieces (`text`, a
 * marked replacement, or an unknown placeholder), and `recorded` lists findings
 * that were only logged — they have no placeholder in the text because the
 * value left unchanged.
 */
export function splitDlp(text, incidents = []) {
  const source = text || '';
  const list = Array.isArray(incidents) ? incidents : [];
  const parts = [];
  const recorded = [];
  let last = 0;

  for (const match of source.matchAll(PLACEHOLDER)) {
    const at = match.index ?? 0;
    if (at > last) parts.push({ kind: 'text', text: source.slice(last, at) });

    const token = match[0];
    const incident = list.find((entry) => entry?.replacement === token);
    if (!incident) {
      parts.push({ kind: 'unknown', text: token });
    } else {
      const kind = KIND_BY_ACTION[incident.action] || 'replaced';
      parts.push({ kind, text: token, incident });
    }
    last = at + token.length;
  }

  if (last < source.length) parts.push({ kind: 'text', text: source.slice(last) });

  for (const incident of list) {
    if (incident?.action === 'allowed' && incident.masked_value) recorded.push(incident);
  }

  return { parts, recorded };
}
