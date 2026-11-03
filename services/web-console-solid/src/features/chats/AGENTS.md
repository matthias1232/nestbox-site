# chats/ — the model conversations of a sandbox, and of the whole environment

## Purpose

The promise: a conversation reads like a messenger thread, live, with the content
check's work visible on the text itself — what was replaced, what was
pseudonymised, what never left because the policy blocked it.

Two sources feed one list. The API stores turns that already happened; the
stream delivers turns while they are still running. `chatsMerge.js` is the only
place that knows how the two become one, and it is pure so a test can pin the
order without a browser.

## Files

| File | Responsibility |
|---|---|
| `ChatBrowser.jsx` | The grouped list (session, model, day, sandbox, owner) beside the thread; filters, search, order, paging, `compact` layout for the workbox; imports `src/styles/chats.css` |
| `ChatThread.jsx` | The messenger view: order, autoscroll, "New messages ↓"; **contract**: props `turns` and optional `title`, a running turn carries `pending: true` |
| `ChatThreadHead.jsx` | Whose conversation this is (sandbox, user, agent, account, models, range) and the two ways out: copy the thread as Markdown, export `.md`/`.json` |
| `ChatTurn.jsx` | One turn: user and model bubbles, tool calls as a collapsible card, tokens, the blocked banner with its route, per-bubble copy, the finding drawer |
| `useChats.js` | State: groups, selection, filters, 400 ms debounced search, older page via `before`, the stream and its teardown on unmount. No JSX |
| `chatsMerge.js` | Pure: `applyEvent` (start → delta → done), `replaceTurn`, `mergeLive`, `sortTurns`, `turnKey`; bounded at `MAX_LIVE_TURNS` |
| `chatExport.js` | Pure text output: `toMarkdown`, `toJson`, `exportFilename`; `downloadText` writes the blob |
| `chatsFormat.js` | Pure wording: times, token counts, model and provider labels, tool calls, `threadHeader`, `groupCounters` |
| `findingsQueue.js` | The bounded loader (four in flight, then a queue, then a cache); a factory so a test drives it with a fake `load` |
| `chatFindings.js` | The one shared queue wired to `api/chats.turn(id)`, so a finding read in one view is there in the other |
| `chatsClipboard.js` | `copyText`, with the selection fallback for a console served over plain HTTP |

## Rules

1. Only masked text is shown and only masked text is exported. A placeholder is
   a link to its finding, never a reason to guess the value.
2. A blocked turn is visibly blocked: red bubble, struck text, the route it would
   have taken, and a way to allow it on purpose (`DlpFindingDrawer`).
3. A running turn is a turn: `pending: true`, the growing answer, no invented
   text, and the typing dots only while there is nothing to show.
4. The stream is closed when the component unmounts; a listener left behind is a
   request per event for the life of the tab.
5. A sandbox view drops events of other sandboxes. A stream that carries
   everything must not put another sandbox's prompt on this screen.
6. Colours come from `styles/dlp.css` (`--chat-user`, `--chat-model`,
   `--chat-tool`, `--dlp-*`), never from a hex value in `chats.css`.
7. `ChatThread`'s contract is fixed: `turns`, optional `title`, `pending` on a
   running turn. F-4 renders the same component; a change here is a change
   there.
8. Every label, tooltip and export line is English, because the console ships
   English (P08). The Markdown export says "**User**" and "**Model**", and a
   blocked turn reads "not delivered to <provider>/<model>". `src/noGerman.test.js`
   fails if German characters or a short word list come back.

## Not this folder's job

`chats.liveWatch('thread')` is F-4's call when a live view is opened; this folder
never records a watch by itself.


## Files added by the Control Center phase

| File | Purpose |
|---|---|
| `chatExport.test.js` | Node tests for `chatExport`. |
| `chatsFormat.test.js` | Node tests for `chatsFormat`. |
| `chatsMerge.test.js` | Node tests for `chatsMerge`. |
| `findingsQueue.test.js` | Node tests for `findingsQueue`. |
