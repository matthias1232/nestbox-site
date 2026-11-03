// The prices the cost columns are computed from, and the models that have none.
//
// Prices are typed in dollars (or whatever the list is quoted in) per million
// tokens and stored as micros. Saving one prices the calls already recorded for
// that model, so the page stops saying n/a for traffic that has a price today.
import { For, Show, createSignal } from 'solid-js';
import * as accounting from '../../api/accounting.js';
import { usePolling } from '../../components/Poll.js';
import { money, tokens } from './format.js';

const toMicros = (text) => {
  const value = Number.parseFloat(String(text).replace(',', '.'));
  return Number.isFinite(value) && value >= 0 ? Math.round(value * 1_000_000) : null;
};

export default function PriceList(props) {
  const [list, setList] = createSignal({ prices: [], unpriced: [] });
  const [error, setError] = createSignal(null);
  const [message, setMessage] = createSignal(null);
  const [draft, setDraft] = createSignal({ model: '', input: '', output: '', cached: '' });
  usePolling(() => accounting.prices(), { everyMs: 30000, onData: (d) => setList(d), onError: (e) => setError(e.message) });

  const fill = (model) => setDraft({ model, input: '', output: '', cached: '' });
  const edit = (price) => setDraft({
    model: price.model,
    input: String(price.input_micros_per_mtok / 1_000_000),
    output: String(price.output_micros_per_mtok / 1_000_000),
    cached: price.cached_micros_per_mtok == null ? '' : String(price.cached_micros_per_mtok / 1_000_000),
  });

  async function save() {
    const d = draft();
    const input = toMicros(d.input);
    const output = toMicros(d.output);
    const cached = d.cached.trim() === '' ? null : toMicros(d.cached);
    if (!d.model.trim() || input == null || output == null || (d.cached.trim() !== '' && cached == null)) {
      setError('A model and non-negative input and output prices are needed.');
      return;
    }
    try {
      await accounting.setPrice({ model: d.model.trim(), input_per_mtok: input, output_per_mtok: output, cached_per_mtok: cached });
      setError(null);
      setMessage(`Price for ${d.model.trim()} saved; earlier calls of this model are priced now.`);
      setDraft({ model: '', input: '', output: '', cached: '' });
      setList(await accounting.prices());
      props.onChanged?.();
    } catch (failure) {
      setError(failure.message);
    }
  }

  return (
    <details style="margin-top:1rem">
      <summary><b>Price list</b> <span class="muted small">{list().prices.length} prices · {list().unpriced.length} models without one</span></summary>
      <Show when={error()}><p class="error">{error()}</p></Show>
      <Show when={message()}><p class="notice">{message()}</p></Show>
      <Show when={list().unpriced.length}>
        <p class="small">Used, but no price (last 30 days) — click to price:
          <For each={list().unpriced}>
            {(item) => <button class="tiny secondary" style="margin-left:.4rem" onClick={() => fill(item.model)}>{item.model} · {tokens(item.calls)} calls</button>}
          </For>
        </p>
      </Show>
      <div class="controls">
        <input placeholder="model" value={draft().model} onInput={(e) => setDraft({ ...draft(), model: e.currentTarget.value })} />
        <input placeholder="input $/Mtok" size="10" value={draft().input} onInput={(e) => setDraft({ ...draft(), input: e.currentTarget.value })} />
        <input placeholder="output $/Mtok" size="10" value={draft().output} onInput={(e) => setDraft({ ...draft(), output: e.currentTarget.value })} />
        <input placeholder="cache read $/Mtok (optional)" size="14" value={draft().cached} onInput={(e) => setDraft({ ...draft(), cached: e.currentTarget.value })} />
        <button class="primary tiny" onClick={save}>Save price</button>
      </div>
      <p class="muted small">Cache writes are priced from the input rate (1.25x for five minutes, 2x for an hour), as Anthropic publishes them; cache reads use the optional cache rate, else the input rate.</p>
      <div class="table-scroll">
        <table class="dense">
          <thead><tr><th>Model</th><th class="num">Input / Mtok</th><th class="num">Output / Mtok</th><th class="num">Cache read / Mtok</th><th>Scope</th><th /></tr></thead>
          <tbody>
            <For each={list().prices} fallback={<tr><td colspan="6" class="muted small">No price yet. Without one, a model's cost is n/a, never 0.</td></tr>}>
              {(price) => (
                <tr>
                  <td class="mono small">{price.model}</td>
                  <td class="num">{money(price.input_micros_per_mtok)}</td>
                  <td class="num">{money(price.output_micros_per_mtok)}</td>
                  <td class="num">{price.cached_micros_per_mtok == null ? '= input' : money(price.cached_micros_per_mtok)}</td>
                  <td>{price.scope}</td>
                  <td><button class="tiny secondary" onClick={() => edit(price)}>Edit</button></td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </details>
  );
}
