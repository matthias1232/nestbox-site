// The chat list and the selected thread.
//
// One component for two places: a sandbox's Chats tab (no `compact`) and the
// workbox panel (`compact`), where the list moves above the thread. The sandbox
// is fixed by the caller; everything else is a filter the reader can change.
import { For, Show } from 'solid-js';
import Segmented from '../../components/Segmented.jsx';
import ChatThread from './ChatThread.jsx';
import { ACCOUNT_KINDS, DLP_STATES, ORDERS, useChats } from './useChats.js';
import { formatTime, formatTokens, groupCounters } from './chatsFormat.js';
import '../../styles/chats.css';

export default function ChatBrowser(props) {
  const chats = useChats({ sandboxId: () => props.sandboxId });

  const isSelected = (group) => {
    const current = chats.selected();
    return Boolean(group) && current?.key === group.key;
  };

  return (
    <section class="chat-browser" classList={{ compact: Boolean(props.compact) }}>
      <aside class="chat-side">
        <div class="chat-side-head">
          <Segmented
            options={chats.groupings()}
            value={chats.by()}
            onChange={(value) => chats.setBy(value)}
          />
          <div class="chat-side-row">
            <input
              type="search"
              class="chat-search"
              placeholder="Suchen…"
              aria-label="Chats durchsuchen"
              value={chats.search()}
              onInput={(event) => chats.changeSearch(event.currentTarget.value)}
            />
            <Segmented
              options={ORDERS}
              value={chats.order()}
              onChange={(value) => chats.setOrder(value)}
            />
            <button
              type="button"
              class="tiny chat-live"
              data-on={chats.liveEnabled() ? 'on' : 'off'}
              title={chats.liveEnabled() ? 'Live stream on' : 'Live stream off'}
              onClick={() => chats.setLiveEnabled(!chats.liveEnabled())}
            >
              Live
            </button>
          </div>
          <div class="chat-filters">
            <input
              type="text"
              placeholder="Modell"
              aria-label="Modell filtern"
              value={chats.model()}
              onInput={(event) => chats.setModel(event.currentTarget.value)}
            />
            <input
              type="text"
              placeholder="Anbieter"
              aria-label="Anbieter filtern"
              value={chats.provider()}
              onInput={(event) => chats.setProvider(event.currentTarget.value)}
            />
            <select
              aria-label="Filter by check"
              value={chats.dlp()}
              onChange={(event) => chats.setDlp(event.currentTarget.value)}
            >
              <For each={DLP_STATES}>
                {(option) => <option value={option.value}>{option.label}</option>}
              </For>
            </select>
            <select
              aria-label="Konto-Art filtern"
              value={chats.accountKind()}
              onChange={(event) => chats.setAccountKind(event.currentTarget.value)}
            >
              <For each={ACCOUNT_KINDS}>
                {(option) => <option value={option.value}>{option.label}</option>}
              </For>
            </select>
          </div>
        </div>

        <Show when={chats.error()}>
          <p class="error small">{chats.error()}</p>
        </Show>

        <ul class="chat-groups">
          <li>
            <button
              type="button"
              class="chat-group chat-group-all"
              classList={{ active: !chats.selected() }}
              onClick={() => chats.select(null)}
            >
              <span class="chat-group-head">
                <span class="chat-group-label">Alle, chronologisch</span>
              </span>
            </button>
          </li>
          <For
            each={chats.groups()}
            fallback={
              <li>
                <p class="muted small chat-empty">
                  {chats.loading() ? 'Loading…' : 'No chats yet.'}
                </p>
              </li>
            }
          >
            {(group) => (
              <li>
                <button
                  type="button"
                  class="chat-group"
                  classList={{ active: isSelected(group) }}
                  title={group.key}
                  onClick={() => chats.select(group)}
                >
                  <span class="chat-group-head">
                    <span class="chat-group-label">{group.label || group.key}</span>
                    <span class="chat-group-count">{formatTokens(group.turns)}</span>
                  </span>
                  <span class="chat-group-meta">
                    <span>{formatTime(group.last_ts)}</span>
                    <span>
                      {formatTokens(group.input_tokens)} ein · {formatTokens(group.output_tokens)} aus
                    </span>
                  </span>
                  <span class="chat-group-dlp">
                    <For each={groupCounters(group)}>
                      {(counter) => (
                        <span
                          class="chat-counter"
                          data-state={counter.state}
                          title={`${counter.label}: ${counter.value}`}
                        >
                          {counter.value} {counter.label}
                        </span>
                      )}
                    </For>
                  </span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </aside>

      <div class="chat-main">
        <ChatThread turns={chats.turns()} title={chats.title()} />
        <Show when={chats.hasMore()}>
          <div class="chat-more">
            <button
              type="button"
              class="secondary tiny"
              disabled={chats.loadingOlder()}
              onClick={() => chats.loadOlder()}
            >
              {chats.loadingOlder() ? 'Loading…' : 'Load older'}
            </button>
          </div>
        </Show>
      </div>
    </section>
  );
}
