// Accounting: what a request used, what it cost, and what a window shows.
//
// `cost_micros` is `null` when no price was known, never `0`: the screen says
// "n/a" instead of drawing a free-looking total. The query builder drops an
// empty sandbox id, so an organisation-wide read carries no filter at all and
// an omitted sandbox is never sent as the string "undefined".

import { query, request } from './client.js';

// The window, and optionally one sandbox inside it. `timeframe` is the server's
// spelling (`5m`, `1h`, `24h`, `7d`, `30d`); it does not accept another.
export const overview = ({ timeframe = '24h', sandbox_id } = {}) =>
  request(`accounting/overview${query({ timeframe, sandbox_id })}`);

// The newest rows of the same window. A separate read on purpose: a window with
// a million requests has a summary the console can draw and a list it cannot.
export const requests = ({ timeframe = '24h', sandbox_id, limit = 100 } = {}) =>
  request(`accounting/requests${query({ timeframe, sandbox_id, limit })}`);

// How long raw rows are kept. Readable by anyone who may read telemetry; the
// write below is the admin half.
export const settings = () => request('accounting/settings');
export const saveSettings = (body) => request('accounting/settings', { method: 'PUT', body });

// Admin: roll the window into the daily table, then delete raw rows older than
// `days`. The response says how many went.
export const purge = (days) => request('accounting/purge', { method: 'POST', body: { days } });

// Usage grouped by one dimension (provider, model, sandbox, auth, owner, site,
// organization, agent, conversation, day): tokens, cache split, hit rate, the spend
// made with API keys, and the list-price estimate of account usage. Kept apart.
export const usage = ({ timeframe = '24h', group_by = 'model', sandbox_id } = {}) =>
  request(`accounting/usage${query({ timeframe, group_by, sandbox_id })}`);

// The price list and the models that were used but have none. Prices are micros
// per million tokens; the console converts from dollars per million.
export const prices = () => request('accounting/prices');
export const setPrice = (body) => request('accounting/prices', { method: 'PUT', body });
