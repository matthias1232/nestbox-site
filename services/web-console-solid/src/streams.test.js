// Guards on the live streams: one state stream and at most one telemetry stream per window.
//
// The console froze on 2026-11-03 because the browser keeps six HTTP/1.1 connections per
// host, shared by every tab and pop-out, and each feature opened its own `EventSource`. Once
// six were open, every further request waited forever and nothing answered. These tests
// keep the two properties that fix it: the demand on a shared stream is reference counted
// (a second screen must not restart it) and `new EventSource` lives in the API modules only.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  BACKOFF_MAX_MS, BACKOFF_START_MS, STATE_EVENTS, TELEMETRY_EVENTS, createRefCount, nextDelay,
} from './state/liveCore.js';

const SRC = fileURLToPath(new URL('.', import.meta.url));

test('a shared stream closes only when the last request is given back', () => {
  const demand = createRefCount();
  const heard = [];
  demand.watch((wanted) => heard.push(wanted));

  const first = demand.acquire();
  const second = demand.acquire();
  assert.equal(demand.count(), 2);
  assert.equal(demand.wanted(), true);

  first();
  assert.equal(demand.count(), 1);
  second();
  assert.equal(demand.count(), 0);
  assert.equal(demand.wanted(), false);
  assert.deepEqual(heard, [true, false], 'the watcher hears the crossing of zero, not every request');
});

test('giving a request back twice does not close a stream someone else holds', () => {
  const demand = createRefCount();
  const first = demand.acquire();
  const second = demand.acquire();

  first();
  first();
  assert.equal(demand.count(), 1, 'a double release is a no-op');
  assert.equal(demand.wanted(), true);

  second();
  assert.equal(demand.count(), 0);
});

test('an unbalanced release never drives the count below zero', () => {
  const demand = createRefCount();
  const release = demand.acquire();
  release();
  release();
  assert.equal(demand.count(), 0);
  assert.equal(demand.wanted(), false);
  // The next request still opens the stream, as if nothing had happened.
  const again = demand.acquire();
  assert.equal(demand.wanted(), true);
  again();
});

test('the reconnect delay starts at 1 s, doubles and never passes 30 s', () => {
  assert.equal(nextDelay(0), BACKOFF_START_MS);
  let delay = 0;
  const steps = [];
  for (let i = 0; i < 8; i += 1) {
    delay = nextDelay(delay);
    steps.push(delay);
  }
  assert.deepEqual(steps, [1000, 2000, 4000, 8000, 16000, 30000, 30000, 30000]);
  assert.ok(steps.every((step) => step <= BACKOFF_MAX_MS));
});

test('a stream that opened again starts its delay over', () => {
  // `keepOpen` resets the delay to 0 on `onOpen`, so the next failure waits one second.
  const afterThreeFailures = nextDelay(nextDelay(nextDelay(0)));
  assert.equal(afterThreeFailures, 4000);
  assert.equal(nextDelay(0), BACKOFF_START_MS);
});

test('the streams carry the event names of contract C18', () => {
  for (const name of ['sandbox', 'gateway', 'job', 'policy_rollout', 'control', 'node',
    'chat_started', 'chat_delta', 'chat_done', 'models', 'alert']) {
    assert.ok(STATE_EVENTS.includes(name), `the state stream must listen for ${name}`);
  }
  assert.deepEqual(TELEMETRY_EVENTS, ['network', 'dns', 'inference']);
});

// Only these modules may name an `EventSource`. `api/events.js` is the opener the shared
// store uses; chats and the agent-situation page still own their streams (a later package
// moves them). Every other file subscribes through `useLive()`.
const STREAM_OWNERS = new Set(['api/events.js', 'api/chats.js', 'api/live.js']);

test('no feature or page opens its own EventSource', () => {
  const offenders = [];
  for (const entry of readdirSync(SRC, { recursive: true })) {
    if (!/\.(js|jsx)$/.test(entry)) continue;
    const file = join(SRC, entry);
    const rel = relative(SRC, file).split(sep).join('/');
    // A test may name the browser API (this one does, in the pattern below).
    if (rel.endsWith('.test.js')) continue;
    if (STREAM_OWNERS.has(rel)) continue;
    if (/\bnew\s+EventSource\b/.test(readFileSync(file, 'utf8'))) offenders.push(rel);
  }
  assert.deepEqual(
    offenders,
    [],
    'subscribe through useLive(): one state stream and at most one telemetry stream per window',
  );
});

// U01: a request the browser opens itself (WebSocket, EventSource, noVNC) carries the chosen
// data center in `?environment=`, because it cannot send the header. Every such call site must
// pass through `environmentUrl`; a new one that forgets it fails here.
test('every browser-opened connection carries the data center', () => {
  const offenders = [];
  for (const entry of readdirSync(SRC, { recursive: true })) {
    if (!/\.(js|jsx)$/.test(entry)) continue;
    const file = join(SRC, entry);
    const rel = relative(SRC, file).split(sep).join('/');
    if (rel.endsWith('.test.js')) continue;
    const lines = readFileSync(file, 'utf8').split('\n');
    lines.forEach((line, index) => {
      if (!/new\s+(WebSocket|EventSource|RFB)\(/.test(line)) return;
      const window = lines.slice(index, index + 3).join(' ');
      if (!window.includes('environmentUrl(')) offenders.push(`${rel}:${index + 1}`);
    });
  }
  assert.deepEqual(offenders, [], 'wrap the URL in environmentUrl(), see api/client.js');
});
