// A thread as a file, or as a paragraph on the clipboard.
//
// Only the masked text is exported, because the masked text is the only text
// the console ever had: there is nothing to redact here, and nothing to leak.

import { formatTime, modelLabel, normaliseToolCalls, toolText } from './chatsFormat.js';

const PAD = (value) => String(value).padStart(2, '0');

function fence(body) {
  // A fence inside the body would end the block early; the longest run decides.
  const longest = Math.max(0, ...(body.match(/`+/g) || []).map((run) => run.length));
  const bar = '`'.repeat(Math.max(3, longest + 1));
  return `${bar}tool\n${body}\n${bar}`;
}

function toolBlock(turn) {
  const calls = normaliseToolCalls(turn?.tool_calls);
  if (!calls.length) return '';
  return fence(calls.map(toolText).join('\n'));
}

/**
 * One thread as Markdown: the question with its time and model, then the answer.
 *
 * A blocked turn says so in the heading; a pending one says it is still growing.
 * A reader of the file must be able to tell the three apart without the console.
 */
export function toMarkdown(turns = []) {
  const blocks = [];
  for (const turn of turns) {
    const model = modelLabel(turn);
    const lines = [];
    if (turn?.user_text) lines.push(`**User** (${formatTime(turn.ts)}, ${model}): ${turn.user_text}`);
    const blocked = turn?.delivered === false;
    const heading = blocked ? '**Model** (blocked, not delivered)' : '**Model**';
    const answer = turn?.assistant_text || (turn?.pending ? '…' : '');
    lines.push(`${heading}: ${answer}`);
    if (blocked) lines.push(`> Not delivered to ${turn?.provider || 'the provider'}/${model}.`);
    const tools = toolBlock(turn);
    if (tools) lines.push(tools);
    if (lines.length) blocks.push(lines.join('\n\n'));
  }
  return blocks.join('\n\n---\n\n');
}

function exportTurn(turn) {
  return {
    id: turn?.id || null,
    ts: turn?.ts || null,
    model: modelLabel(turn),
    provider: turn?.provider || null,
    sandbox: turn?.sandbox_name || turn?.sandbox_id || null,
    owner: turn?.owner_name || turn?.owner_user_id || null,
    agent: turn?.agent || null,
    session_id: turn?.session_id || null,
    request_ref: turn?.request_ref || null,
    user_text: turn?.user_text || '',
    assistant_text: turn?.assistant_text || '',
    tool_calls: normaliseToolCalls(turn?.tool_calls),
    delivered: turn?.delivered !== false,
    pending: Boolean(turn?.pending),
    dlp: turn?.dlp || null,
    tokens: {
      input: Number(turn?.input_tokens) || 0,
      output: Number(turn?.output_tokens) || 0,
      cache_read: Number(turn?.cache_read) || 0,
      cache_write: Number(turn?.cache_write) || 0,
    },
  };
}

/** The same thread as JSON, one object per turn, in display order. */
export function toJson(turns = []) {
  return JSON.stringify(turns.map(exportTurn), null, 2);
}

/** A filename that sorts by time and names the console that wrote it. */
export function exportFilename(format = 'md', at = new Date()) {
  const stamp = `${at.getFullYear()}${PAD(at.getMonth() + 1)}${PAD(at.getDate())}-${PAD(at.getHours())}${PAD(at.getMinutes())}`;
  return `nestbox-chat-${stamp}.${format === 'json' ? 'json' : 'md'}`;
}

/**
 * Save a text as a file, from a browser.
 *
 * Returns false where there is no document, so a test can call the exporters
 * without a DOM and a worker does not throw on a button it cannot press.
 */
export function downloadText(text, filename, mime = 'text/plain') {
  if (typeof document === 'undefined' || typeof URL === 'undefined') return false;
  const blob = new Blob([text], { type: `${mime};charset=utf-8` });
  const url = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  link.rel = 'noopener';
  document.body.append(link);
  link.click();
  link.remove();
  // The click reads the blob; revoking in the same tick cancels the download.
  setTimeout(() => URL.revokeObjectURL(url), 0);
  return true;
}
