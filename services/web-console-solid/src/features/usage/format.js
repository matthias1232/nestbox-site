// Display rules for the usage views. Pure functions.
//
// Two rules matter. A missing price is "n/a", never 0: zero reads as "free" and a
// total built from free-looking rows cannot be defended. And what an API key cost
// is never added to what an account's usage would cost at list price: they are
// different things, so they have different columns and different totals.

const MICROS = 1_000_000;

/** 1_234 -> "1.2k", 3_400_000 -> "3.4M"; a number the server did not give is a dash. */
export function tokens(value) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  if (abs >= 1_000_000_000) return `${(value / 1_000_000_000).toFixed(2)}B`;
  if (abs >= 1_000_000) return `${(value / 1_000_000).toFixed(2)}M`;
  if (abs >= 10_000) return `${(value / 1_000).toFixed(1)}k`;
  return value.toLocaleString();
}

/** Micros as an amount; `null` means no price was known. */
export function money(micros) {
  if (typeof micros !== 'number' || !Number.isFinite(micros)) return 'n/a';
  const units = micros / MICROS;
  if (units === 0) return '0.00';
  if (units >= 1) return units.toFixed(2);
  if (units >= 0.01) return units.toFixed(4);
  return units.toFixed(6);
}

/** 0.9438 -> "94.4 %". */
export function percent(rate) {
  if (typeof rate !== 'number' || !Number.isFinite(rate)) return '—';
  return `${(rate * 100).toFixed(1)} %`;
}

export const GROUPS = [
  { id: 'model', label: 'Model' },
  { id: 'provider', label: 'Provider' },
  { id: 'sandbox', label: 'Sandbox' },
  { id: 'auth', label: 'Key or account' },
  { id: 'owner', label: 'Owner' },
  { id: 'site', label: 'Data center / site' },
  { id: 'organization', label: 'Customer (organization)' },
  { id: 'agent', label: 'Agent' },
  { id: 'conversation', label: 'Conversation' },
  { id: 'day', label: 'Day' },
];

export const FRAMES = ['1h', '24h', '7d', '30d'];

/** The label of a key: `auth` keys get words, the rest are shown as they are. */
export function keyLabel(group, key) {
  if (group === 'auth') {
    return { key: 'API key', account: 'Signed-in account', unknown: 'Unknown' }[key] || key;
  }
  if (group === 'conversation' && key && key.length > 12) return `${key.slice(0, 8)}…`;
  return key;
}
