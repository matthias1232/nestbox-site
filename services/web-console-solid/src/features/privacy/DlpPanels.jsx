// The DLP dashboard, the log and the policy history, shared by the admin page,
// the sandbox tab and the end user's card.
//
// What the numbers mean: an incident is one finding by one detector. Its
// `action` says what the layer did — refused the request (`blocked`), put stars
// over it (`redacted`), replaced the whole finding with a placeholder
// (`replaced`), or only recorded it (`allowed`). `requested_action` says what the
// policy asked for, which differs in `observe`: there the policy asks for a
// rewrite and the request goes out untouched, and the row says "would have".
//
// "From → to" is the masked value and the substitute the filter wrote. Neither
// is the content: the value itself is never stored, so it cannot be shown here,
// by anyone.
import { For, Show, createSignal } from "solid-js";
import * as privacy from "../../api/privacy.js";
import { usePolling } from "../../components/Poll.js";

const FRAMES = ["1h", "24h", "7d", "30d"];
const GROUPS = [
  { id: "detector", label: "Detector" },
  { id: "action", label: "What happened" },
  { id: "severity", label: "Severity" },
  { id: "sandbox", label: "Sandbox" },
  { id: "day", label: "Day" },
  { id: "destination", label: "Destination" },
];
const ACTIONS = ["all", "blocked", "redacted", "replaced", "allowed"];
const DIRECTIONS = [
  { id: "all", label: "both directions" },
  { id: "inbound", label: "to the model" },
  { id: "outbound", label: "to the agent" },
];
const SOURCES = [
  { id: "all", label: "every hop" },
  { id: "egress", label: "egress proxy" },
  { id: "accounting_proxy", label: "accounting proxy" },
  { id: "mcp", label: "MCP call" },
  { id: "test", label: "test" },
];

const num = (value) =>
  typeof value === "number" ? value.toLocaleString() : "—";

export const actionWord = (key) => {
  if (key === "blocked") return "refused";
  if (key === "redacted") return "masked";
  if (key === "replaced") return "replaced";
  if (key === "allowed") return "recorded only";
  return String(key || "").replaceAll("_", " ");
};

export const sourceWord = (key) => {
  if (key === "egress") return "egress proxy";
  if (key === "accounting_proxy") return "accounting proxy";
  if (key === "mcp") return "MCP call";
  if (key === "test") return "test";
  return key;
};

const directionWord = (key) =>
  key === "outbound"
    ? "to the agent"
    : key === "inbound"
      ? "to the model"
      : "—";

// What happened, in one badge: the effective action, plus "would have" when the
// policy asked for more than the mode allowed.
function ActionBadge(props) {
  const effective = () => props.incident.action;
  const asked = () => props.incident.requested_action;
  const differs = () => asked() && asked() !== effective();
  return (
    <span
      class={`badge ${effective() === "blocked" ? "error" : effective() === "allowed" ? "muted" : "warn"}`}
      title={
        differs() ? `The policy asked for: ${actionWord(asked())}` : undefined
      }
    >
      <i />
      {actionWord(effective())}
      <Show when={differs()}>
        <span class="muted small"> · would have {actionWord(asked())}</span>
      </Show>
    </span>
  );
}

export function DlpDashboard(props) {
  const [frame, setFrame] = createSignal("24h");
  const [group, setGroup] = createSignal("detector");
  const [report, setReport] = createSignal(null);
  const [error, setError] = createSignal(null);
  const poll = usePolling(
    () =>
      privacy.overview({
        timeframe: frame(),
        group_by: group(),
        sandbox_id: props.sandboxId,
      }),
    {
      everyMs: 15000,
      onData: (data) => {
        setReport(data);
        setError(null);
      },
      onError: (e) => setError(e.message),
    },
  );
  const totals = () => report()?.totals;
  const rows = () => report()?.rows || [];
  const peak = () => Math.max(1, ...rows().map((row) => row.incidents));

  return (
    <section class="tab-panel">
      <div class="controls">
        <button
          class={poll.paused() ? "secondary tiny" : "primary tiny"}
          onClick={poll.toggle}
        >
          {poll.paused() ? "Paused" : "Live"}
        </button>
        <span class="muted small">group by</span>
        <select
          value={group()}
          onChange={(e) => {
            setGroup(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <For each={GROUPS}>
            {(item) => <option value={item.id}>{item.label}</option>}
          </For>
        </select>
        <span class="muted small">last</span>
        <select
          value={frame()}
          onChange={(e) => {
            setFrame(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <For each={FRAMES}>
            {(item) => <option value={item}>{item}</option>}
          </For>
        </select>
      </div>
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
      <Show when={totals()}>
        <div class="controls" style="gap:1.5rem;flex-wrap:wrap">
          <span>
            <b>{num(totals().incidents)}</b>{" "}
            <span class="muted small">findings</span>
          </span>
          <span>
            <b>{num(totals().blocked)}</b>{" "}
            <span class="muted small">requests refused</span>
          </span>
          <span>
            <b>{num(totals().redacted)}</b>{" "}
            <span class="muted small">masked</span>
          </span>
          <span>
            <b>{num(totals().replaced)}</b>{" "}
            <span class="muted small">replaced</span>
          </span>
          <span>
            <b>{num(totals().allowed)}</b>{" "}
            <span class="muted small">recorded only</span>
          </span>
          <span>
            <b>{num(totals().cloud)}</b>{" "}
            <span class="muted small">towards a cloud destination</span>
          </span>
        </div>
      </Show>
      <div class="table-scroll">
        <table class="dense">
          <thead>
            <tr>
              <th>{GROUPS.find((g) => g.id === group())?.label}</th>
              <th class="num">Findings</th>
              <th class="num">Refused</th>
              <th class="num">Masked</th>
              <th class="num">Recorded</th>
              <th class="num">Cloud</th>
              <th />
            </tr>
          </thead>
          <tbody>
            <For
              each={rows()}
              fallback={
                <tr>
                  <td colspan="7" class="muted small">
                    No finding in this window. That is the good case; it is not
                    proof that nothing would be found — the layer reports what
                    its detectors match.
                  </td>
                </tr>
              }
            >
              {(row) => (
                <tr>
                  <td class="mono small">
                    {group() === "action" ? actionWord(row.key) : row.key}
                  </td>
                  <td class="num">{num(row.incidents)}</td>
                  <td class="num">{num(row.blocked)}</td>
                  <td class="num">{num(row.redacted)}</td>
                  <td class="num">{num(row.allowed)}</td>
                  <td class="num">{num(row.cloud)}</td>
                  <td style="width:90px">
                    <div
                      style={`height:6px;border-radius:3px;background:var(--chart-1,#60a5fa);width:${Math.max(2, Math.round((row.incidents / peak()) * 100))}%`}
                    />
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </section>
  );
}

// Where a finding happened, in the words of the request: host and path for the
// proxy, the tool name for an MCP call.
function Target(props) {
  const incident = () => props.incident;
  return (
    <span class="small muted">
      {incident().mcp_tool ? `tool ${incident().mcp_tool}` : ""}
      {incident().mcp_tool && incident().destination ? " · " : ""}
      {incident().destination || ""}
      {incident().path ? <span class="mono"> {incident().path}</span> : ""}
      {incident().model ? (
        <span>
          {" "}
          · <span class="mono">{incident().model}</span>
        </span>
      ) : (
        ""
      )}
    </span>
  );
}

export function DlpLog(props) {
  const [frame, setFrame] = createSignal("24h");
  const [action, setAction] = createSignal("all");
  const [direction, setDirection] = createSignal("all");
  const [source, setSource] = createSignal("all");
  const [detector, setDetector] = createSignal("all");
  const [items, setItems] = createSignal([]);
  const [error, setError] = createSignal(null);
  const poll = usePolling(
    () =>
      privacy.log({
        timeframe: frame(),
        sandbox_id: props.sandboxId,
        action: action(),
        direction: direction(),
        source: source(),
        detector: detector(),
        limit: props.limit || 100,
      }),
    {
      everyMs: 10000,
      onData: (data) => {
        setItems(data.items || []);
        setError(null);
      },
      onError: (e) => setError(e.message),
    },
  );
  const detectors = () =>
    [...new Set(items().map((item) => item.incident.detector))].sort();

  return (
    <section class="tab-panel">
      <div class="controls">
        <button
          class={poll.paused() ? "secondary tiny" : "primary tiny"}
          onClick={poll.toggle}
        >
          {poll.paused() ? "Paused" : "Live"}
        </button>
        <span class="muted small">what happened</span>
        <select
          value={action()}
          onChange={(e) => {
            setAction(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <For each={ACTIONS}>
            {(item) => (
              <option value={item}>
                {item === "all" ? "all" : actionWord(item)}
              </option>
            )}
          </For>
        </select>
        <span class="muted small">direction</span>
        <select
          value={direction()}
          onChange={(e) => {
            setDirection(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <For each={DIRECTIONS}>
            {(item) => <option value={item.id}>{item.label}</option>}
          </For>
        </select>
        <span class="muted small">seen by</span>
        <select
          value={source()}
          onChange={(e) => {
            setSource(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <For each={SOURCES}>
            {(item) => <option value={item.id}>{item.label}</option>}
          </For>
        </select>
        <span class="muted small">detector</span>
        <select
          value={detector()}
          onChange={(e) => {
            setDetector(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <option value="all">all</option>
          <For each={detectors()}>
            {(item) => <option value={item}>{actionWord(item)}</option>}
          </For>
        </select>
        <span class="muted small">last</span>
        <select
          value={frame()}
          onChange={(e) => {
            setFrame(e.currentTarget.value);
            poll.refresh();
          }}
        >
          <For each={FRAMES}>
            {(item) => <option value={item}>{item}</option>}
          </For>
        </select>
      </div>
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
      <div class="table-scroll">
        <table class="dense">
          <thead>
            <tr>
              <th>When</th>
              <th>Direction</th>
              <th>Detector</th>
              <th>What happened</th>
              <th>From → to</th>
              <th>Request</th>
              <th>Seen by</th>
              <Show when={!props.sandboxId}>
                <th>Sandbox</th>
              </Show>
            </tr>
          </thead>
          <tbody>
            <For
              each={items()}
              fallback={
                <tr>
                  <td colspan={props.sandboxId ? 7 : 8} class="muted small">
                    No finding in this window.
                  </td>
                </tr>
              }
            >
              {(item) => (
                <tr>
                  <td class="mono small">
                    {String(item.incident.created_at).slice(11, 19)}
                  </td>
                  <td class="small">
                    {directionWord(item.incident.direction)}
                  </td>
                  <td>{actionWord(item.incident.detector)}</td>
                  <td>
                    <ActionBadge incident={item.incident} />
                  </td>
                  <td class="mono small">
                    <span title="the value, masked: the first and last characters are all that is kept">
                      {item.incident.masked_value || "—"}
                    </span>
                    <Show when={item.incident.replacement}>
                      <span class="muted"> → </span>
                      <span title="what the filter wrote instead">
                        {item.incident.replacement}
                      </span>
                    </Show>
                  </td>
                  <td>
                    <Target incident={item.incident} />
                  </td>
                  <td class="small muted">
                    {sourceWord(item.incident.source)}
                  </td>
                  <Show when={!props.sandboxId}>
                    <td class="mono small">{item.sandbox_name}</td>
                  </Show>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
      <p class="muted small">
        Only the masked excerpt and the substitute are stored. The value a
        detector matched is never kept, so it cannot be shown here — not by an
        administrator either.
      </p>
    </section>
  );
}

// One before/after pair from the stored summary. Only the fields that changed
// are printed, because a list of eight identical values is not a diff.
function ChangeDiff(props) {
  const before = () => props.before || {};
  const after = () => props.after || {};
  const keys = [
    "mode",
    "detectors",
    "block_cloud_egress",
    "retention_days",
    "overrides",
  ];
  const value = (source, key) => {
    const raw = source[key];
    if (raw === undefined || raw === null) return "—";
    if (Array.isArray(raw)) return raw.length ? raw.join(", ") : "none";
    return String(raw);
  };
  const changed = () =>
    keys.filter((key) => value(before(), key) !== value(after(), key));
  return (
    <Show
      when={changed().length}
      fallback={
        <span class="muted small">saved without a change to the values</span>
      }
    >
      <For each={changed()}>
        {(key) => (
          <div class="small">
            <span class="mono">{key}</span>
            <span class="muted"> {value(before(), key)}</span>
            <span class="muted"> → </span>
            <b>{value(after(), key)}</b>
          </div>
        )}
      </For>
    </Show>
  );
}

export function DlpChanges(props) {
  const [items, setItems] = createSignal([]);
  const [error, setError] = createSignal(null);
  const poll = usePolling(
    () =>
      privacy.changes({
        sandbox_id: props.sandboxId,
        limit: props.limit || 50,
      }),
    {
      everyMs: 30000,
      onData: (data) => {
        setItems(data.items || []);
        setError(null);
      },
      onError: (e) => setError(e.message),
    },
  );
  return (
    <section class="tab-panel">
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
      <p class="muted small">
        A change to this policy is an audited act. A sandbox inherits the
        environment default until it saves its own row; both histories are
        shown, because "why did it stop blocking" is usually a change one level
        up.
      </p>
      <div class="table-scroll">
        <table class="dense">
          <thead>
            <tr>
              <th>When</th>
              <th>Who</th>
              <th>What</th>
              <th>Scope</th>
              <th>Change</th>
            </tr>
          </thead>
          <tbody>
            <For
              each={items()}
              fallback={
                <tr>
                  <td colspan="5" class="muted small">
                    Nothing recorded in this window.
                  </td>
                </tr>
              }
            >
              {(item) => (
                <tr>
                  <td class="mono small">
                    {String(item.created_at).replace("T", " ").slice(0, 19)}
                  </td>
                  <td class="small">{item.actor_label}</td>
                  <td class="small">
                    {item.kind === "privacy.inherit"
                      ? "reset to inherit"
                      : item.kind === "privacy.purge"
                        ? "purged incidents"
                        : "saved the policy"}
                  </td>
                  <td class="small muted">
                    {item.subject_type === "environment"
                      ? "environment default"
                      : "this sandbox"}
                  </td>
                  <td>
                    <Show
                      when={item.kind === "privacy.settings"}
                      fallback={
                        <span class="muted small">
                          {JSON.stringify(item.detail)}
                        </span>
                      }
                    >
                      <ChangeDiff
                        before={item.detail?.before}
                        after={item.detail?.after}
                      />
                    </Show>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
      <div class="controls">
        <button class="secondary tiny" onClick={poll.refresh}>
          Refresh
        </button>
      </div>
    </section>
  );
}
