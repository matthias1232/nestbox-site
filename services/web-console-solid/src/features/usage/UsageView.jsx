// What the models were asked and what it used, grouped by whatever the reader
// wants: model, provider, sandbox, owner, data center, customer, day.
//
// One component for both places it lives: the whole environment (the "Usage" page)
// and one sandbox (a tab of its deck). The numbers come from the proxy that reads
// the answers, so they cover signed-in accounts (Claude, Codex) as well as API keys.
// The two are kept apart on purpose: a key has a cost; an account has an estimate at
// list price, which is shown on its own and never added to the cost.
import { For, Show, createSignal } from 'solid-js';
import * as accounting from '../../api/accounting.js';
import { usePolling } from '../../components/Poll.js';
import { FRAMES, GROUPS, keyLabel, money, percent, tokens } from './format.js';
import PriceList from './PriceList.jsx';

export default function UsageView(props) {
  const [report, setReport] = createSignal(null);
  const [error, setError] = createSignal(null);
  const [frame, setFrame] = createSignal('24h');
  const [group, setGroup] = createSignal(props.defaultGroup || 'model');
  const poll = usePolling(
    () => accounting.usage({ timeframe: frame(), group_by: group(), sandbox_id: props.sandboxId }),
    { everyMs: 15000, onData: (d) => { setReport(d); setError(null); }, onError: (e) => setError(e.message) },
  );
  const totals = () => report()?.totals;
  const rows = () => report()?.rows || [];
  const peak = () => Math.max(1, ...rows().map((row) => row.prompt_tokens + row.completion_tokens));
  const groups = () => (props.sandboxId ? GROUPS.filter((item) => item.id !== 'sandbox') : GROUPS);

  return (
    <section class="tab-panel">
      <div class="controls">
        <button class={poll.paused() ? 'secondary tiny' : 'primary tiny'} onClick={poll.toggle}>{poll.paused() ? 'Paused' : 'Live'}</button>
        <span class="muted small">group by</span>
        <select value={group()} onChange={(e) => { setGroup(e.currentTarget.value); poll.refresh(); }}>
          <For each={groups()}>{(item) => <option value={item.id}>{item.label}</option>}</For>
        </select>
        <span class="muted small">last</span>
        <select value={frame()} onChange={(e) => { setFrame(e.currentTarget.value); poll.refresh(); }}>
          <For each={FRAMES}>{(item) => <option value={item}>{item}</option>}</For>
        </select>
      </div>
      <Show when={error()}><p class="error">{error()}</p></Show>

      <Show when={totals()}>
        <div class="controls" style="gap:1.5rem;flex-wrap:wrap">
          <span><b>{tokens(totals().requests)}</b> <span class="muted small">calls</span></span>
          <span><b>{tokens(totals().prompt_tokens)}</b> <span class="muted small">read</span></span>
          <span><b>{tokens(totals().completion_tokens)}</b> <span class="muted small">written</span></span>
          <span title="Cache reads as a share of everything the models read"><b>{percent(totals().cache_hit_rate)}</b> <span class="muted small">cache hit</span></span>
          <span><b>{tokens(totals().cache_read_tokens)}</b> <span class="muted small">cache reads</span></span>
          <span><b>{tokens(totals().cache_write_tokens)}</b> <span class="muted small">cache writes ({tokens(totals().cache_write_1h_tokens)} for 1 h)</span></span>
          <span><b>{totals().cache_markers}</b> <span class="muted small">cache_control markers</span></span>
        </div>
        <div class="controls" style="gap:1.5rem;flex-wrap:wrap">
          <span title="API-key calls, priced from the price list"><b>{money(totals().cost_micros)}</b> <span class="muted small">spent with API keys ({totals().key_requests} calls)</span></span>
          <span title="What these calls would cost at list price. The subscription is what is actually paid; this is not added to the spend."><b>{money(totals().estimated_cost_micros)}</b> <span class="muted small">account usage at list price, estimate ({totals().account_requests} calls)</span></span>
          <Show when={totals().unpriced_requests > 0}>
            <span class="muted small">{totals().unpriced_requests} calls have no price; add the model to the price list to see their cost.</span>
          </Show>
        </div>
      </Show>

      <div class="table-scroll">
        <table class="dense">
          <thead>
            <tr>
              <th>{GROUPS.find((item) => item.id === group())?.label}</th>
              <th class="num">Calls</th><th class="num">Read</th><th class="num">Written</th>
              <th class="num">Cache reads</th><th class="num">Cache writes</th><th>Cache hit</th>
              <th class="num">API-key spend</th><th class="num">Account (est.)</th><th></th>
            </tr>
          </thead>
          <tbody>
            <For each={rows()} fallback={<tr><td colspan="10" class="muted small">No model call with counters in this window. Counters come from the proxy reading the answers, so the sandbox needs an inspecting egress mode (transparent or hybrid), or its agent must use the accounting proxy.</td></tr>}>
              {(row) => (
                <tr>
                  <td class="mono small">{keyLabel(group(), row.key)}</td>
                  <td class="num">{tokens(row.requests)}</td>
                  <td class="num">{tokens(row.prompt_tokens)}</td>
                  <td class="num">{tokens(row.completion_tokens)}</td>
                  <td class="num">{tokens(row.cache_read_tokens)}</td>
                  <td class="num">{tokens(row.cache_write_tokens)}</td>
                  <td>
                    <div style="display:flex;align-items:center;gap:.4rem">
                      <div style="width:70px;height:6px;background:var(--border);border-radius:3px;overflow:hidden">
                        <div style={`width:${Math.round(row.cache_hit_rate * 100)}%;height:100%;background:var(--chart-2, #4ade80)`} />
                      </div>
                      <span class="small">{percent(row.cache_hit_rate)}</span>
                    </div>
                  </td>
                  <td class="num">{money(row.cost_micros)}</td>
                  <td class="num muted">{money(row.estimated_cost_micros)}</td>
                  <td style="width:90px"><div style={`height:6px;border-radius:3px;background:var(--chart-1, #60a5fa);width:${Math.max(2, Math.round(((row.prompt_tokens + row.completion_tokens) / peak()) * 100))}%`} /></td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
      <Show when={!props.sandboxId}><PriceList onChanged={poll.refresh} /></Show>
      <p class="muted small">
        Read = everything the model read (fresh input + cache reads + cache writes). Spend is only for calls
        made with an API key; a signed-in account (Claude, Codex) is shown as an estimate at list price and is
        not part of the spend. A price of n/a means the model has no entry in the price list.
      </p>
    </section>
  );
}
