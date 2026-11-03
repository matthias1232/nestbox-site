// Turn a recorded request/response pair into a chat turn.
//
// The proxy stores raw bodies (at most 64 KiB, base64 on the wire). Which of them
// are model calls is decided here, from the path, because a host list would be
// wrong the day a gateway or a self-hosted model appears. Three dialects are
// read: Anthropic messages, OpenAI chat completions, and OpenAI responses.
// Streamed answers (server-sent events) are stitched back into text.

const LLM_PATH = /\/(v1\/messages|v1\/chat\/completions|chat\/completions|v1\/responses|responses)(\?|$)/;

/** True when a recorded request looks like a model call. */
export function isModelCall(event) {
  return event?.method === 'POST' && LLM_PATH.test(event?.path || '');
}

function textOf(content) {
  if (content == null) return '';
  if (typeof content === 'string') return content;
  if (Array.isArray(content)) {
    return content
      .map((part) => {
        if (typeof part === 'string') return part;
        if (part.type === 'text' || part.type === 'input_text' || part.type === 'output_text') return part.text || '';
        if (part.type === 'tool_use') return `[tool call ${part.name}(${JSON.stringify(part.input ?? {}).slice(0, 400)})]`;
        if (part.type === 'tool_result') return `[tool result ${textOf(part.content).slice(0, 400)}]`;
        if (part.type === 'image' || part.type === 'image_url') return '[image]';
        return '';
      })
      .filter(Boolean)
      .join('\n');
  }
  return JSON.stringify(content);
}

function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

// A request cut at the size limit is not valid JSON. The proxy keeps the start
// and the end of a long request, so the model sits in the head and the newest
// message in the tail; both can still be read with a pattern.
function salvageRequest(text) {
  const model = /"model"\s*:\s*"([^"]+)"/.exec(text)?.[1];
  const users = [...text.matchAll(/"role"\s*:\s*"user"\s*,\s*"content"\s*:\s*"((?:[^"\\]|\\.)*)"/g)];
  const last = users.length ? users[users.length - 1][1] : '';
  let prompt = last;
  try {
    prompt = JSON.parse(`"${last}"`);
  } catch {
    /* keep the escaped text */
  }
  return { model, prompt, partial: true };
}

/** The id of the conversation a call belongs to, when the client names one. */
export function sessionOf(text) {
  if (!text) return null;
  const direct = /"nestbox_conversation"\s*:\s*"([0-9a-f-]{8,})"/.exec(text);
  if (direct) return direct[1];
  // Claude Code: `metadata.user_id` is a JSON string, so its quotes are escaped.
  const escaped = /session_id\\?"\s*:\s*\\?"([0-9a-f-]{8,})/.exec(text);
  return escaped ? escaped[1] : null;
}

/** A title the client asked the model to write for a conversation, if this answer is one. */
export function titleOf(answerText) {
  const match = /^\s*\{\s*"title"\s*:\s*"((?:[^"\\]|\\.)*)"/.exec(answerText || '');
  if (!match) return null;
  try {
    return JSON.parse(`"${match[1]}"`);
  } catch {
    return match[1];
  }
}

/** What was asked: model, the newest user message, and how long the history was. */
export function readRequest(text) {
  if (!text) return { model: null, prompt: '', history: 0 };
  const body = parseJson(text);
  if (!body) return { ...salvageRequest(text), history: 0 };
  const messages = body.messages || (Array.isArray(body.input) ? body.input : []);
  const users = messages.filter((message) => (message.role || 'user') === 'user');
  const newest = users.length ? users[users.length - 1] : null;
  const input = typeof body.input === 'string' ? body.input : '';
  return {
    model: body.model || null,
    system: textOf(body.system) || undefined,
    prompt: newest ? textOf(newest.content) : input,
    history: body.nestbox_history ?? messages.length,
    stream: body.stream === true,
    tools: Array.isArray(body.tools) ? body.tools.length : 0,
  };
}

/** What came back: the assistant text, tool calls, and token counts when present. */
export function readResponse(text) {
  if (!text) return { text: '', tokens: null };
  const whole = parseJson(text);
  if (whole) {
    if (whole.error) return { text: '', error: whole.error.message || JSON.stringify(whole.error), tokens: null };
    const usage = whole.usage || {};
    const tokens = usage.input_tokens != null || usage.prompt_tokens != null
      ? { input: usage.input_tokens ?? usage.prompt_tokens, output: usage.output_tokens ?? usage.completion_tokens }
      : null;
    if (Array.isArray(whole.content)) return { text: textOf(whole.content), tokens };
    if (Array.isArray(whole.choices)) return { text: textOf(whole.choices[0]?.message?.content), tokens };
    if (Array.isArray(whole.output)) return { text: whole.output.map((item) => textOf(item.content)).join('\n'), tokens };
  }
  // Server-sent events: one `data:` JSON per line.
  let out = '';
  let tokens = null;
  for (const line of text.split('\n')) {
    if (!line.startsWith('data:')) continue;
    const payload = parseJson(line.slice(5).trim());
    if (!payload) continue;
    if (payload.type === 'content_block_delta') out += payload.delta?.text || '';
    else if (payload.type === 'message_start' && payload.message?.usage) {
      tokens = { input: payload.message.usage.input_tokens, output: payload.message.usage.output_tokens };
    } else if (payload.type === 'message_delta' && payload.usage) {
      tokens = { input: tokens?.input, output: payload.usage.output_tokens };
    } else if (payload.choices) out += payload.choices[0]?.delta?.content || '';
    else if (payload.type === 'response.output_text.delta') out += payload.delta || '';
  }
  return { text: out, tokens };
}
