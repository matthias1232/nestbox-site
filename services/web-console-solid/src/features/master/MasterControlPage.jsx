// Master control: the organisation's accounting window, one sandbox drilled
// down, and the retention policy behind it.
//
// The screen answers "what did we spend, on which model, in which sandbox, and
// what did the content filter do on the way?" for the window the operator picks.
// It renders the server's numbers and never adds them up itself: a total
// computed here would eventually disagree with the one the server can defend.

import { For, Show, createMemo, onMount } from 'solid-js';
import Empty from '../../components/Empty.jsx';
import BarList from './BarList.jsx';
import { TIMEFRAMES, useMasterControl } from './useMasterControl.js';
import PageHeader from '../shell/PageHeader.jsx';
import HelpTip from '../../components/HelpTip.jsx';
import {
  costLabel,
  count,
  latencyLabel,
  missingPriceCount,
  privacyRows,
  privacyTone,
  rowCost,
  shortId,
  statusTone,
  whenLabel,
} from './accountingFormat.js';

// An endpoint can be long; the table scrolls, but one long path must not push
// every other column off the screen.
const ENDPOINT = {
  maxWidth: '260px',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  whiteSpace: 'nowrap',
};

export default function MasterControlPage(props) {
  const ctl = useMasterControl();
  const isAdmin = () => props.role === 'admin';

  onMount(() => {
    ctl.load();
    ctl.loadSettings();
  });

  const totals = () => ctl.overview()?.totals || null;
  const cost = createMemo(() => costLabel(totals()));
  const missing = createMemo(() => missingPriceCount(totals()));
  const privacy = createMemo(() => privacyRows(ctl.overview()?.privacy));
  const privacyTotal = createMemo(() => privacy().reduce((sum, row) => sum + row.count, 0));

  const frameHint = () =>
    TIMEFRAMES.find((frame) => frame.id === ctl.timeframe())?.hint || 'the selected window';
  const scopeName = () => ctl.sandboxName() || shortId(ctl.sandbox());

  // The bars are requests; the figures on the right are the cost those requests
  // carried. A group with no price says n/a rather than drawing a free row.
  const modelRows = () =>
    (ctl.overview()?.by_model || []).map((group) => {
      const name = group.model || 'unknown model';
      const money = rowCost(group);
      return {
        label: name,
        value: group.requests,
        meta: `${count(group.requests)} req · ${money}`,
        title: `${name}: ${count(group.requests)} requests, cost ${money}`,
      };
    });

  const sandboxRows = () =>
    (ctl.overview()?.by_sandbox || []).map((group) => {
      const name = group.sandbox_name || shortId(group.sandbox_id);
      return {
        label: name,
        value: group.requests,
        meta: `${count(group.requests)} req · ${rowCost(group)}`,
        title: `Show only ${name}`,
        onSelect: () => ctl.drillDown(group.sandbox_id, name),
      };
    });

  return (
    <section class="page">
      <PageHeader
        title="Master control"
        subtitle={<>What this organisation's sandboxes used and spent in one window, by model and by sandbox. Numbers come from the accounting layer; a request with no price is shown as n/a, never as free.</>}
        actions={<>
          <button class="secondary" onClick={() => ctl.load()} disabled={ctl.loading()}>
            {ctl.loading() ? 'Refreshing…' : 'Refresh'}
          </button>
          <HelpTip anchor="getting-started" />
        </>}
      />

      <Show when={ctl.error()}>
        <p class="error">{ctl.error()}</p>
      </Show>
      <Show when={ctl.notice()}>
        <p class="notice">{ctl.notice()}</p>
      </Show>

      <div class="facets" role="group" aria-label="Timeframe">
        <For each={TIMEFRAMES}>
          {(frame) => (
            <button
              class={ctl.timeframe() === frame.id ? 'chip active' : 'chip'}
              aria-pressed={ctl.timeframe() === frame.id}
              title={`Show ${frame.hint}`}
              onClick={() => ctl.chooseTimeframe(frame.id)}
            >
              {frame.label}
            </button>
          )}
        </For>
      </div>

      <Show when={ctl.sandbox()}>
        <div class="bulkbar">
          <span>
            Drill-down: <b>{scopeName()}</b> only. The totals below are this sandbox's, not
            the organisation's.
          </span>
          <button class="tiny secondary" onClick={() => ctl.clearDrillDown()}>
            Clear drill-down
          </button>
        </div>
      </Show>

      <Show
        when={ctl.overview()}
        fallback={
          <p class="muted">
            {ctl.loading() ? 'Loading accounting…' : 'No accounting overview for this window.'}
          </p>
        }
      >
        <div class="cards">
          <article>
            <span>Requests</span>
            <b>{count(totals()?.requests)}</b>
            <small>over the {frameHint()}</small>
          </article>
          <article>
            <span>Input tokens</span>
            <b>{count(totals()?.prompt_tokens)}</b>
            <small>prompt tokens, cached ones included</small>
          </article>
          <article>
            <span>Output tokens</span>
            <b>{count(totals()?.completion_tokens)}</b>
            <small>completion tokens, reasoning included</small>
          </article>
          <article>
            <span>Cache-read tokens</span>
            <b>{count(totals()?.cached_tokens)}</b>
            <small>the part of the input the provider served from its cache</small>
          </article>
          <article>
            <span>Cost</span>
            <b class={cost().unknown ? 'bad-text' : undefined}>{cost().text}</b>
            <small>{cost().detail}</small>
          </article>
        </div>

        <Show when={totals() && totals().requests === 0}>
          <p class="muted small">
            No request was recorded in this window. The counts above are zero because nothing
            was recorded, not because something was left out.
          </p>
        </Show>

        <div class="split">
          <section class="editor">
            <h3>By model</h3>
            <p class="muted small">
              The bar is requests; the figure on the right is the cost those requests carried.
              A model whose requests had no price says n/a.
            </p>
            <Show when={missing() > 0}>
              <p class="warn-note small">
                {count(missing())} of {count(totals()?.requests)} requests in this window had no
                price. The window cost therefore says n/a instead of a sum that would leave them
                out.
              </p>
            </Show>
            <BarList rows={modelRows()} empty="No model was called in this window." />
          </section>

          <section class="editor">
            <h3>By sandbox</h3>
            <p class="muted small">
              Choose a sandbox to narrow the whole screen to it: its own totals, its models and
              its request rows.
            </p>
            <BarList
              rows={sandboxRows()}
              empty={
                ctl.sandbox()
                  ? 'A sandbox-scoped view has no per-sandbox list. Clear the drill-down to compare sandboxes.'
                  : 'No sandbox made a request in this window.'
              }
            />
          </section>
        </div>

        <section class="editor">
          <h3>Content privacy</h3>
          <p class="muted small">
            What the filter did with content on its way in or out in this window. A request
            without a decision was not inspected; that is not the same as a clean request.
          </p>
          <Show when={privacyTotal() === 0}>
            <p class="muted small">The server recorded no privacy decision in this window.</p>
          </Show>
          <div class="table-scroll">
            <table class="table dense">
              <thead>
                <tr>
                  <th class="narrow">Action</th>
                  <th class="narrow">Count</th>
                  <th>What it means</th>
                </tr>
              </thead>
              <tbody>
                <For each={privacy()}>
                  {(row) => (
                    <tr>
                      <td class="narrow">
                        <span class={`badge ${row.tone}`}>
                          <i />
                          {row.action}
                        </span>
                      </td>
                      <td class="narrow mono small">{count(row.count)}</td>
                      <td class="small muted">{row.explanation}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </section>

        <section class="editor">
          <div class="draft-head">
            <div>
              <h3>Requests</h3>
              <p class="muted small" style={{ margin: 0 }}>
                Newest first, at most {ctl.REQUEST_LIMIT} rows,{' '}
                {ctl.sandbox() ? `for ${scopeName()}` : 'across this environment'}. The list is
                a page of the window, not all of it.
              </p>
            </div>
            <Show when={ctl.sandbox()}>
              <button class="tiny secondary" onClick={() => ctl.clearDrillDown()}>
                Clear drill-down
              </button>
            </Show>
          </div>

          <Show when={ctl.requestsError()}>
            <p class="error small">{ctl.requestsError()}</p>
          </Show>

          <Show
            when={!ctl.requestsLoading()}
            fallback={<p class="muted small">Loading requests…</p>}
          >
            <Show
              when={ctl.requests().length}
              fallback={
                <Empty
                  title="No requests in this window"
                  hint={
                    ctl.sandbox()
                      ? 'This sandbox recorded no request in the selected window. Pick a longer timeframe if you expected traffic.'
                      : 'No sandbox recorded a request in the selected window. Pick a longer timeframe if you expected traffic.'
                  }
                />
              }
            >
              <div class="table-scroll">
                <table class="table dense">
                  <thead>
                    <tr>
                      <th>When</th>
                      <th>Sandbox</th>
                      <th>Agent</th>
                      <th>Provider / model</th>
                      <th>Endpoint</th>
                      <th class="narrow">Status</th>
                      <th class="narrow">Latency</th>
                      <th>Tokens in / out / cache</th>
                      <th class="narrow">Cost</th>
                      <th>Privacy</th>
                    </tr>
                  </thead>
                  <tbody>
                    <For each={ctl.requests()}>
                      {(row) => (
                        <tr>
                          <td class="small">{whenLabel(row.created_at)}</td>
                          <td class="small">
                            {row.sandbox_name || shortId(row.sandbox_id)}
                          </td>
                          <td class="small mono">{row.agent_instance || '—'}</td>
                          <td class="small">
                            <span class="mono">{row.provider || '—'}</span>
                            <span class="muted"> / </span>
                            <span class="mono">{row.model || '—'}</span>
                          </td>
                          <td class="small">
                            <div
                              class="mono"
                              style={ENDPOINT}
                              title={row.endpoint || undefined}
                            >
                              {row.endpoint || '—'}
                            </div>
                          </td>
                          <td class="narrow">
                            <Show
                              when={typeof row.status === 'number'}
                              fallback={<span class="muted">—</span>}
                            >
                              <span class={`badge ${statusTone(row.status)}`} title="HTTP status">
                                <i />
                                {row.status}
                              </span>
                            </Show>
                          </td>
                          <td class="narrow mono small">{latencyLabel(row.latency_ms)}</td>
                          <td
                            class="mono small"
                            title={`prompt ${count(row.prompt_tokens)}, completion ${count(
                              row.completion_tokens,
                            )}, cached ${count(row.cached_tokens)}`}
                          >
                            {count(row.prompt_tokens)} / {count(row.completion_tokens)} /{' '}
                            {count(row.cached_tokens)}
                          </td>
                          <td
                            class={
                              typeof row.cost_micros === 'number'
                                ? 'narrow mono small'
                                : 'narrow mono small muted'
                            }
                            title={
                              row.price_source ? `price source: ${row.price_source}` : undefined
                            }
                          >
                            {rowCost(row)}
                          </td>
                          <td class="small">
                            <Show when={row.privacy_action} fallback={<span class="muted">—</span>}>
                              <span
                                class={`badge ${privacyTone(row.privacy_action)}`}
                                title={row.privacy_detector || row.privacy_action}
                              >
                                <i />
                                {row.privacy_action}
                              </span>
                              <Show when={row.privacy_detector}>
                                <div class="muted small mono">{row.privacy_detector}</div>
                              </Show>
                            </Show>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </div>
            </Show>
          </Show>
        </section>
      </Show>

      <section class="editor">
        <h3>Retention</h3>

        <Show when={ctl.settingsError()}>
          <p class="error small">{ctl.settingsError()}</p>
        </Show>

        <Show
          when={ctl.settings()}
          fallback={
            <p class="muted small">
              {ctl.settingsLoading()
                ? 'Loading retention settings…'
                : 'No retention setting was returned for this environment.'}
            </p>
          }
        >
          <Show
            when={isAdmin()}
            fallback={
              <p class="muted small">
                Raw requests are kept for {count(ctl.draft().retention_days)} days; daily rollups
                are {ctl.draft().keep_daily ? 'kept' : 'not kept'} after a purge. Only an admin can
                change this or purge.
              </p>
            }
          >
            <div class="form-grid">
              <label class="field">
                Keep raw requests for (days)
                <input
                  class="input"
                  type="number"
                  min="1"
                  max="3650"
                  value={ctl.draft().retention_days}
                  disabled={ctl.saving()}
                  onInput={(event) => ctl.setRetentionDays(event.currentTarget.value)}
                />
              </label>
              <div class="field">
                <span>Daily rollups</span>
                <label class="small">
                  <input
                    type="checkbox"
                    checked={ctl.draft().keep_daily}
                    disabled={ctl.saving()}
                    onChange={(event) => ctl.setKeepDaily(event.currentTarget.checked)}
                  />{' '}
                  Keep a daily rollup after the raw rows are purged
                </label>
              </div>
            </div>
            <button class="primary" disabled={ctl.saving()} onClick={() => ctl.saveSettings()}>
              {ctl.saving() ? 'Saving…' : 'Save retention'}
            </button>

            <h4>Purge</h4>
            <p class="muted small">
              Deletes raw request rows older than the day count. When daily rollups are on, the
              window is folded into the daily table first; the rollup itself is never deleted
              here.
            </p>
            <div class="form-grid">
              <label class="field">
                Purge raw requests older than (days)
                <input
                  class="input"
                  type="number"
                  min="1"
                  max="3650"
                  value={ctl.purgeDays()}
                  disabled={ctl.purging()}
                  onInput={(event) => ctl.setPurgeDayCount(event.currentTarget.value)}
                />
              </label>
              <Show
                when={ctl.pendingPurge()}
                fallback={
                  <div class="controls">
                    <button
                      class="tiny danger"
                      disabled={ctl.purging()}
                      onClick={() => ctl.askPurge()}
                    >
                      Purge raw requests
                    </button>
                  </div>
                }
              >
                <div class="controls">
                  <button
                    class="tiny danger"
                    disabled={ctl.purging()}
                    onClick={() => ctl.confirmPurge()}
                  >
                    Delete raw rows older than {ctl.purgeDays()} days — cannot be undone
                  </button>
                  <button
                    class="tiny secondary"
                    disabled={ctl.purging()}
                    onClick={() => ctl.cancelPurge()}
                  >
                    Cancel
                  </button>
                </div>
              </Show>
            </div>
          </Show>
        </Show>
      </section>
    </section>
  );
}
