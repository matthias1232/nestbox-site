// A clicking smoke suite over the whole console, one scenario per page function.
//
// Dev tool only: it is not part of the bundle. It drives one real headless Chromium over CDP,
// signs in once with LOGIN_EMAIL/LOGIN_PASSWORD, opens the `default` data center, then runs the
// scenarios in ./smoke-scenarios.json against http://192.168.55.50:8080 (or SMOKE_BASE).
//
// Every scenario is { name, path, wait?, login?, steps }, and every step is exactly one of
//   { "click": "<css>" }        click the first element that matches
//   { "type": "<text>" }        insert text at the focus (CDP Input.insertText)
//   { "key": "Enter" }          press a key or a combo ("Enter", "Escape", "ctrl+k")
//   { "wait": <ms> }            wait
//   { "expectText": "<s>" }     the visible page text contains s
//   { "expectNoText": "<s>" }   it does not
//   { "expectSelector": "<css>"} at least one element matches
//   { "eval": "<js>" }          the expression returns true (a promise is awaited)
//
// Tests find bugs, so this runner never repairs a page: a FAIL is reported, a screenshot is
// written to /tmp/cc-smoke/<name>.png, and the process exits 1. Console errors are listed; an
// uncaught exception in a scenario fails it, because a page that throws is not a working page.
//
// Signing in as a second user (scenario 20) needs MEMBER_EMAIL/MEMBER_PASSWORD; a scenario with
// `"login": "member"` gets that session and does not give the first one back.
//
// A scenario whose page comes up as the sign-in form (the session was lost between two pages)
// signs in once with the same account and opens the page again; the line says RESIGNIN, so a
// recovered session is visible and never counted as a page that worked.
//
// Usage: node scripts/smoke.mjs [scenario-name ...]   (no names = all, in file order)
// env: LOGIN_EMAIL, LOGIN_PASSWORD, MEMBER_EMAIL, MEMBER_PASSWORD, SMOKE_BASE, SMOKE_OUT,
//      CHROME (path to the headless binary), SMOKE_STOP=1 (stop at the first failing step),
//      SMOKE_RETRY=0 (do not retry a page the browser never rendered), SMOKE_DEBUG=1 (cookies,
//      Set-Cookie headers, per-step state)
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';

const BASE = process.env.SMOKE_BASE || 'http://192.168.55.50:8080';
const OUT = process.env.SMOKE_OUT || '/tmp/cc-smoke';
const ORIGIN = new URL(BASE).origin;
// The full Chromium keeps the session cookie; the headless shell build drops it mid-run on this
// box (every request then answers 401, which looks like twenty page bugs and is none).
const BIN = process.env.CHROME
  || [
    '/home/core/.cache/ms-playwright/chromium-1148/chrome-linux/chrome',
    '/home/core/.cache/ms-playwright/chromium_headless_shell-1148/chrome-linux/headless_shell',
  ].find((candidate) => existsSync(candidate))
  || 'chromium';

// The data center every scenario starts in (the one with the nodes and the sandboxes).
const ENV_DEFAULT = '00000000-0000-0000-0000-000000000002';

// The live objects the plan names, so a scenario file stays readable. The SMOKE_* env vars
// override them when the box changed.
const VARS = {
  '{{desktop01}}': process.env.SMOKE_DESKTOP || '02d0aa5e-8377-479c-9e47-71e0178a4f8d',
  '{{desktopOS}}': process.env.SMOKE_DESKTOP_OS || 'd4e9bfe1-4965-428c-8245-1c83049fb8b0',
  '{{node}}': process.env.SMOKE_NODE || '00000000-0000-0000-0000-000000000003',
  '{{default}}': ENV_DEFAULT,
  '{{berlin}}': process.env.SMOKE_BERLIN || '55eb109b-8f9e-4e5d-8e8c-dc832d1f519e',
};

const fill = (text) => String(text).replace(/\{\{[A-Za-z0-9_]+\}\}/g, (name) => VARS[name] ?? name);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

mkdirSync(OUT, { recursive: true });
const scenarios = JSON.parse(readFileSync(new URL('./smoke-scenarios.json', import.meta.url), 'utf8'));
const only = process.argv.slice(2).filter((arg) => !arg.startsWith('-'));
// A scenario is named by its full name (`14-traffic`) or by its number (`14`), so "run 14 and
// 18" works as written.
const chosen = only.length
  ? scenarios.filter((scenario) => only.some((name) => scenario.name === name || scenario.name.startsWith(`${name}-`)))
  : scenarios;
if (!chosen.length) {
  console.log(`No scenario matches ${only.join(', ')}. Known: ${scenarios.map((s) => s.name).join(', ')}`);
  process.exit(2);
}

// ---- one headless Chromium, driven over CDP (the connection part of shot.mjs) ----------------
const port = 9400 + Math.floor(Math.random() * 400);
const chrome = spawn(BIN, [
  '--no-sandbox', '--disable-gpu', '--headless', '--hide-scrollbars',
  `--remote-debugging-port=${port}`, '--window-size=1440,900',
  `--user-data-dir=/tmp/nestbox-smoke-${port}`, 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });

let version = null;
for (let i = 0; i < 40 && !version; i += 1) {
  try {
    version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  } catch {
    await sleep(250);
  }
}
if (!version) {
  console.log('RESULT chrome did not start');
  chrome.kill('SIGKILL');
  process.exit(2);
}

const ws = new WebSocket(version.webSocketDebuggerUrl);
await new Promise((resolve) => { ws.onopen = resolve; });
let id = 0;
const pending = new Map();
const events = [];
ws.onmessage = (message) => {
  const data = JSON.parse(message.data);
  if (data.id && pending.has(data.id)) {
    pending.get(data.id)(data);
    pending.delete(data.id);
  } else if (data.method) {
    events.push(data);
  }
};

function send(method, params = {}, sessionId, timeout = 30000) {
  const next = ++id;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      pending.delete(next);
      reject(new Error(`TIMEOUT ${method}`));
    }, timeout);
    pending.set(next, (data) => {
      clearTimeout(timer);
      resolve(data.result || data.error);
    });
    ws.send(JSON.stringify({ id: next, method, params, sessionId }));
  });
}

const { targetId: firstTarget } = await send('Target.createTarget', { url: 'about:blank' });
const { sessionId: firstSession } = await send('Target.attachToTarget', { targetId: firstTarget, flatten: true });
let targetId = firstTarget;
let sessionId = firstSession;
const s = (method, params, timeout) => send(method, params, sessionId, timeout);
await s('Runtime.enable');
await s('Page.enable');
await s('Network.enable');

/**
 * Open a fresh tab for a scenario and close the previous one.
 *
 * One tab per page load is not politeness: a console page holds live streams open, and several
 * leftover streams on one host fill the browser's per-host connection budget, after which the
 * next page sits on "Loading…" forever. A new tab starts clean.
 */
async function freshTab() {
  const previous = targetId;
  const created = await send('Target.createTarget', { url: 'about:blank' });
  targetId = created.targetId;
  const attached = await send('Target.attachToTarget', { targetId, flatten: true });
  sessionId = attached.sessionId;
  await s('Runtime.enable');
  await s('Page.enable');
  await s('Network.enable');
  await s('Network.setCacheDisabled', { cacheDisabled: true });
  // Only now that the new tab exists: closing the last tab would end the browser.
  if (previous) await send('Target.closeTarget', { targetId: previous }).catch(() => {});
}

/** Evaluate one expression in the page; anything but a truthy answer is a failure. */
async function evaluate(expression, { awaitPromise = true, timeout = 20000 } = {}) {
  const answer = await s('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise,
    userGesture: true,
  }, timeout);
  if (answer?.exceptionDetails) {
    const detail = answer.exceptionDetails;
    throw new Error(detail.exception?.description || detail.text || 'the expression threw');
  }
  return answer?.result?.value;
}

/** Navigate and wait for the load event of that navigation (a hung renderer throws). */
async function open(path) {
  const mark = events.length;
  await s('Page.navigate', { url: new URL(path, BASE).href }, 20000);
  for (let i = 0; i < 80; i += 1) {
    await sleep(250);
    if (events.slice(mark).some((event) => event.method === 'Page.loadEventFired')) return;
  }
  throw new Error(`the page did not finish loading: ${path}`);
}

/** Sign in through the API; the cookie stays in the profile for the following navigations. */
async function signIn(email, password) {
  if (!email || !password) throw new Error('a sign-in needs an email and a password');
  await open('/favicon.ico');
  await sleep(500);
  const answer = await evaluate(
    `fetch('/api/v1/auth/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({email:${JSON.stringify(email)},password:${JSON.stringify(password)}})}).then(r=>r.json().then(j=>r.status+':'+(j.csrf_token||'')))`,
  );
  const [status, csrf] = String(answer || '').split(':');
  console.log(status === '200' ? 'LOGIN ok' : `LOGIN status ${status}`);
  if (status !== '200') throw new Error(`sign-in failed with ${status}`);
  // The first-run tour would cover the page; mark it seen once per user, like a person clicking
  // through it. A failure only costs one overlay, so it is not fatal.
  if (csrf) {
    await evaluate(
      `fetch('/api/v1/users/me/tours/shell-v1',{method:'PUT',headers:{'x-csrf-token':${JSON.stringify(csrf)}}}).then(()=>true).catch(()=>true)`,
    ).catch(() => {});
  }
}

/** Open the home page and make `default` the active data center (through the context select). */
async function pinDefaultEnvironment() {
  await open('/');
  await sleep(2500);
  for (let attempt = 0; attempt < 6; attempt += 1) {
    const state = await evaluate(`(() => {
      const select = document.querySelector('.cc-context-bar select');
      if (!select) return 'missing';
      if (select.value === ${JSON.stringify(ENV_DEFAULT)}) return 'ok';
      const option = [...select.options].find((entry) => entry.value === ${JSON.stringify(ENV_DEFAULT)});
      if (!option) return 'no-option';
      select.value = ${JSON.stringify(ENV_DEFAULT)};
      select.dispatchEvent(new Event('change', { bubbles: true }));
      return 'switched';
    })()`);
    if (state === 'ok' || state === 'switched') {
      await sleep(1500);
      return;
    }
    await sleep(1000);
  }
  throw new Error('the data center select never offered `default`');
}

/** Press a key or a combo ("Enter", "Escape", "ctrl+k"). */
async function press(combo) {
  const parts = String(combo).toLowerCase().split('+').map((part) => part.trim()).filter(Boolean);
  const name = parts.pop() || 'enter';
  const modifiers = (parts.includes('alt') ? 1 : 0)
    | (parts.includes('ctrl') || parts.includes('control') ? 2 : 0)
    | (parts.includes('meta') || parts.includes('win') ? 4 : 0)
    | (parts.includes('shift') ? 8 : 0);
  const named = {
    enter: { key: 'Enter', code: 'Enter', vk: 13, text: '\r' },
    escape: { key: 'Escape', code: 'Escape', vk: 27 },
    tab: { key: 'Tab', code: 'Tab', vk: 9 },
    arrowdown: { key: 'ArrowDown', code: 'ArrowDown', vk: 40 },
    arrowup: { key: 'ArrowUp', code: 'ArrowUp', vk: 38 },
  }[name];
  const single = name.length === 1 ? { key: name, code: `Key${name.toUpperCase()}`, vk: name.toUpperCase().charCodeAt(0) } : null;
  const key = named || single;
  if (!key) throw new Error(`unknown key ${combo}`);
  await s('Input.dispatchKeyEvent', {
    type: 'keyDown', modifiers, key: key.key, code: key.code,
    windowsVirtualKeyCode: key.vk, nativeVirtualKeyCode: key.vk,
    ...(key.text ? { text: key.text } : {}),
  });
  await s('Input.dispatchKeyEvent', {
    type: 'keyUp', modifiers, key: key.key, code: key.code,
    windowsVirtualKeyCode: key.vk, nativeVirtualKeyCode: key.vk,
  });
}

const describe = (step) => {
  const [kind] = Object.keys(step);
  const value = step[kind];
  return `${kind} ${typeof value === 'string' ? value.slice(0, 90) : value}`;
};

/** Run one step. Answers { ok, reason }. Never throws for a page-side miss. */
async function runStep(step) {
  if ('click' in step) {
    const hit = await evaluate(`(() => {
      const el = document.querySelector(${JSON.stringify(step.click)});
      if (!el) return 'missing';
      el.click();
      return 'clicked';
    })()`);
    return hit === 'clicked' ? { ok: true } : { ok: false, reason: 'no element matches' };
  }
  if ('type' in step) {
    await s('Input.insertText', { text: String(step.type) });
    return { ok: true };
  }
  if ('key' in step) {
    await press(step.key);
    return { ok: true };
  }
  if ('wait' in step) {
    await sleep(Number(step.wait) || 0);
    return { ok: true };
  }
  if ('expectText' in step) {
    const text = await evaluate('document.body ? document.body.innerText : ""');
    return String(text).includes(step.expectText)
      ? { ok: true }
      : { ok: false, reason: `"${step.expectText}" is not visible` };
  }
  if ('expectNoText' in step) {
    const text = await evaluate('document.body ? document.body.innerText : ""');
    return String(text).includes(step.expectNoText)
      ? { ok: false, reason: `"${step.expectNoText}" is still visible` }
      : { ok: true };
  }
  if ('expectSelector' in step) {
    const found = await evaluate(`!!document.querySelector(${JSON.stringify(step.expectSelector)})`);
    return found ? { ok: true } : { ok: false, reason: 'no element matches' };
  }
  if ('eval' in step) {
    const value = await evaluate(step.eval);
    return value === true ? { ok: true } : { ok: false, reason: `returned ${JSON.stringify(value)}` };
  }
  return { ok: false, reason: `unknown step ${JSON.stringify(step)}` };
}

// ---- the run ---------------------------------------------------------------------------------
const table = [];
let chromeGone = false;

/**
 * Run one scenario and collect its step results, screenshot and diagnostics.
 *
 * Answers everything the report needs for one attempt; the caller decides whether a second
 * attempt is worth it.
 */
async function attemptScenario(scenario) {
  const name = scenario.name;
  const steps = (scenario.steps || []).map((step) => Object.fromEntries(
    Object.entries(step).map(([kind, value]) => [kind, typeof value === 'string' ? fill(value) : value]),
  ));
  const path = fill(scenario.path || '/');
  if (scenario.login === 'member') {
    if (!process.env.MEMBER_EMAIL || !process.env.MEMBER_PASSWORD) {
      throw new Error('scenario 20 needs MEMBER_EMAIL and MEMBER_PASSWORD');
    }
    await signIn(process.env.MEMBER_EMAIL, process.env.MEMBER_PASSWORD);
  }
  const mark = events.length;
  const rows = [];
  if (process.env.SMOKE_DEBUG) {
    const jar = await s('Network.getCookies', { urls: [BASE] }, 5000);
    console.log(`COOKIES ${JSON.stringify((jar?.cookies || []).map((cookie) => ({ name: cookie.name, expires: cookie.expires, session: cookie.session, size: cookie.size })))}`);
  }
  console.log(`\nSCENARIO ${name} ${path}`);
  try {
    await freshTab();
    await open(path);
    await sleep(Number(scenario.wait) || 2000);
    // A browser that shares its box with other work can lose its session between two pages.
    // That is not a page failure: sign in again with the same account and open the page once
    // more, and say so, so nobody mistakes a recovered session for a clean pass.
    const signedOut = await evaluate("!!document.querySelector('form input[type=\"password\"]') && !document.querySelector('.cc-topbar')");
    if (signedOut) {
      console.log('RESIGNIN the page asked for a sign-in again');
      await signIn(
        scenario.login === 'member' ? process.env.MEMBER_EMAIL : process.env.LOGIN_EMAIL,
        scenario.login === 'member' ? process.env.MEMBER_PASSWORD : process.env.LOGIN_PASSWORD,
      );
      if (scenario.login !== 'member') await pinDefaultEnvironment();
      await open(path);
      await sleep(Number(scenario.wait) || 2000);
    }
    for (const [index, step] of steps.entries()) {
      let outcome;
      try {
        outcome = await runStep(step);
      } catch (failure) {
        outcome = { ok: false, reason: failure.message };
      }
      rows.push({ index: index + 1, step, ...outcome });
      console.log(`${outcome.ok ? 'PASS' : 'FAIL'} ${name} ${index + 1} ${describe(step)}${outcome.ok ? '' : ` — ${outcome.reason}`}`);
      if (process.env.SMOKE_DEBUG) {
        const jar = await s('Network.getCookies', { urls: [BASE] }, 5000);
        const where = await evaluate('location.href').catch(() => '?');
        console.log(`  COOKIE after step ${index + 1}: ${(jar?.cookies || []).map((cookie) => cookie.name).join(',') || 'none'} at ${where}`);
      }
      if (!outcome.ok && process.env.SMOKE_STOP) break;
    }
  } catch (failure) {
    rows.push({ index: 0, step: { open: path }, ok: false, reason: failure.message });
    console.log(`FAIL ${name} 0 open ${path} — ${failure.message}`);
  }

  const shot = `${OUT}/${name}.png`;
  try {
    const captured = await s('Page.captureScreenshot', { format: 'png' }, 15000);
    writeFileSync(shot, Buffer.from(captured.data, 'base64'));
    console.log(`SHOT ${shot}`);
  } catch (failure) {
    console.log(`SHOT failed: ${failure.message}`);
  }

  const exceptions = events.slice(mark)
    .filter((event) => event.method === 'Runtime.exceptionThrown')
    .map((event) => event.params.exceptionDetails.exception?.description || event.params.exceptionDetails.text);
  for (const exception of exceptions) {
    rows.push({ index: rows.length + 1, step: { exception: '' }, ok: false, reason: exception });
    console.log(`FAIL ${name} exception — ${String(exception).split('\n')[0]}`);
  }
  const consoleErrors = events.slice(mark)
    .filter((event) => event.method === 'Runtime.consoleAPICalled' && event.params.type === 'error')
    .map((event) => event.params.args.map((arg) => arg.value ?? arg.description ?? '').join(' '));
  const badResponses = events.slice(mark)
    .filter((event) => event.method === 'Network.responseReceived' && event.params.response.status >= 400)
    .map((event) => ({ requestId: event.params.requestId, line: `${event.params.response.status} ${event.params.response.url.replace(ORIGIN, '').split('?')[0]}` }));
  for (const [seen, entry] of [...new Map(badResponses.map((row) => [row.line, row])).entries()]) {
    // The error code names the cause ("session_expired" vs. "unauthenticated"); it is the one
    // thing a maintainer needs from a failing API call, so keep it next to the status.
    let code = '';
    try {
      const body = await s('Network.getResponseBody', { requestId: entry.requestId }, 5000);
      const answer = JSON.parse(body?.body || '{}');
      code = answer.error?.code || answer.code || '';
    } catch { /* the body is a bonus */ }
    console.log(`HTTP ${seen}${code ? ` ${code}` : ''}`);
  }
  // A page that still shows its loading fallback never handed the scenario a screen: the
  // browser could not fetch a lazily imported chunk. That is a harness accident on a busy box,
  // not a page that lost its buttons, so it is worth exactly one more attempt.
  if (process.env.SMOKE_DEBUG) {
    const urls = new Map();
    for (const event of events.slice(mark)) {
      if (event.method === 'Network.requestWillBeSent') urls.set(event.params.requestId, event.params.request.url.replace(ORIGIN, ''));
    }
    for (const event of events.slice(mark)) {
      if (event.method !== 'Network.loadingFailed') continue;
      const info = event.params;
      console.log(`  DEBUG failed ${urls.get(info.requestId) || info.requestId} ${info.type} ${info.errorText}${info.canceled ? ' canceled' : ''}${info.blockedReason ? ` blocked=${info.blockedReason}` : ''}`);
    }
  }
  const stuck = await evaluate('!document.querySelector(".cc-page-header") && document.body.innerText.includes("Loading…")').catch(() => false);
  return {
    name,
    path,
    shot,
    rows,
    passed: rows.every((row) => row.ok),
    exceptionLines: exceptions.map((exception) => String(exception)),
    consoleErrors,
    badResponses: [...new Set(badResponses.map((row) => row.line))],
    loading: stuck === true,
  };
}

try {
  if (!process.env.LOGIN_EMAIL) throw new Error('LOGIN_EMAIL is missing');
  await signIn(process.env.LOGIN_EMAIL, process.env.LOGIN_PASSWORD);
  await pinDefaultEnvironment();

  for (const scenario of chosen) {
    let attempt = await attemptScenario(scenario);
    const harness = attempt.loading
      || attempt.exceptionLines.some((line) => line.includes('Failed to fetch dynamically imported module'))
      || attempt.rows.some((row) => /TIMEOUT|did not finish loading/.test(String(row.reason || '')));
    if (!attempt.passed && harness && process.env.SMOKE_RETRY !== '0') {
      console.log(`RETRY ${scenario.name} once: the page never rendered (harness accident)`);
      attempt = await attemptScenario(scenario);
    }
    table.push(attempt);
  }
} catch (failure) {
  console.log(`\nSMOKE ABORTED: ${failure.message}`);
  process.exitCode = 2;
} finally {
  const width = Math.max(16, ...table.map((row) => row.name.length));
  console.log('\nSMOKE RESULT');
  for (const row of table) {
    const failed = row.rows.filter((entry) => !entry.ok).map((entry) => entry.index).join(',') || '-';
    console.log(`${row.name.padEnd(width)}  ${row.passed ? 'PASS' : 'FAIL'}  steps ${row.rows.every((entry) => entry.ok) ? 'all passed' : `${failed} failed`}  api errors ${row.badResponses.length}  console errors ${row.consoleErrors.length}  ${row.shot}`);
  }
  const bad = table.filter((row) => !row.passed);
  console.log(`\n${table.length - bad.length}/${table.length} scenarios passed`);
  try { ws.close(); } catch { /* already gone */ }
  if (!chromeGone) {
    chrome.kill('SIGKILL');
    chromeGone = true;
  }
  process.exit(bad.length === 0 && chosen.length === table.length ? 0 : 1);
}
