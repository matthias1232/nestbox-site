// The head of a thread: whose conversation it is, which account and which models
// answered, and the two ways out of the screen (clipboard, file).
//
// Everything shown here is derived from the turns on screen. A thread is a
// range, not a session object, so the header must not claim more than it sees.
import { For, Show, createSignal } from 'solid-js';
import { copyText } from './chatsClipboard.js';
import { downloadText, exportFilename, toJson, toMarkdown } from './chatExport.js';
import { formatTime, threadHeader } from './chatsFormat.js';

function Fact(props) {
  return (
    <Show when={props.value}>
      <span class="chat-fact">
        <span class="chat-fact-label">{props.label}</span>
        <span class="chat-fact-value" title={props.title || undefined}>{props.value}</span>
      </span>
    </Show>
  );
}

export default function ChatThreadHead(props) {
  const head = () => threadHeader(props.turns || []);
  const [copied, setCopied] = createSignal(false);
  const [format, setFormat] = createSignal('md');

  async function copyThread() {
    const ok = await copyText(toMarkdown(props.turns || []));
    if (!ok) return;
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  }

  function save() {
    const turns = props.turns || [];
    if (format() === 'json') {
      downloadText(toJson(turns), exportFilename('json'), 'application/json');
      return;
    }
    downloadText(toMarkdown(turns), exportFilename('md'), 'text/markdown');
  }

  return (
    <header class="chat-head">
      <div class="chat-facts">
        <span class="chat-fact">
          <span class="chat-fact-label">Anfragen</span>
          <span class="chat-fact-value">{head().count}</span>
        </span>
        <Fact label="Sandbox" value={head().sandboxes.join(', ')} />
        <Fact label="User" value={head().users.join(', ')} />
        <Fact label="Agent" value={head().agents.join(', ')} />
        <Fact label="Konto" value={head().accounts.join(', ')} />
        <Fact label="Model" value={head().models.join(', ')} />
        <Fact
          label="Zeitraum"
          value={head().first ? `${formatTime(head().first)} – ${formatTime(head().last)}` : null}
        />
      </div>

      <div class="chat-head-actions">
        <button type="button" class="tiny secondary" onClick={copyThread}>
          {copied() ? 'kopiert' : 'Verlauf kopieren'}
        </button>
        <div class="chat-export">
          <select
            aria-label="Exportformat"
            value={format()}
            onChange={(event) => setFormat(event.currentTarget.value)}
          >
            <option value="md">.md</option>
            <option value="json">.json</option>
          </select>
          <button type="button" class="tiny secondary" onClick={save}>Export</button>
        </div>
      </div>
    </header>
  );
}
