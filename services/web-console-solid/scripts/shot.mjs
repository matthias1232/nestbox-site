// Screenshot one console page with headless Chromium over CDP.
//
// Dev tool only: it is not part of the bundle. It proves that a page renders (a hung renderer
// answers no CDP call and exits 1), prints the visible text, the requests, the JS exceptions and
// the console errors, and writes a PNG for a human or a model to look at.
//
// node scripts/shot.mjs <url> <out.png> [waitMs] [clicks]
//   clicks: ';'-separated CSS selectors clicked in order, one second apart.
// env: LOGIN_EMAIL, LOGIN_PASSWORD (optional; logs in through /api/v1/auth/login first),
//      CHROME (optional path to the headless binary).
import { spawn } from 'node:child_process';
import { writeFileSync } from 'node:fs';

const [,, url, out, waitArg, clicksArg] = process.argv;
if (!url || !out) {
  console.log('usage: node scripts/shot.mjs <url> <out.png> [waitMs] [clicks]');
  process.exit(2);
}
const wait = Number(waitArg || 4000);
const clicks = String(clicksArg || '').split(';').map((s) => s.trim()).filter(Boolean);
const BIN = process.env.CHROME
  || '/home/core/.cache/ms-playwright/chromium_headless_shell-1148/chrome-linux/headless_shell';
const port = 9400 + Math.floor(Math.random() * 400);
const chrome = spawn(BIN, [
  '--no-sandbox', '--disable-gpu', '--headless', '--hide-scrollbars',
  `--remote-debugging-port=${port}`, '--window-size=1440,900',
  `--user-data-dir=/tmp/nestbox-shot-${port}`, 'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
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

function send(method, params = {}, sessionId, timeout = 20000) {
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

const { targetId } = await send('Target.createTarget', { url: 'about:blank' });
const { sessionId } = await send('Target.attachToTarget', { targetId, flatten: true });
const s = (method, params, timeout) => send(method, params, sessionId, timeout);
await s('Runtime.enable');
await s('Page.enable');
await s('Network.enable');
const origin = new URL(url).origin;

if (process.env.LOGIN_EMAIL) {
  await s('Page.navigate', { url: `${origin}/favicon.ico` });
  await sleep(800);
  const answer = await s('Runtime.evaluate', {
    expression: `fetch('/api/v1/auth/login',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({email:${JSON.stringify(process.env.LOGIN_EMAIL)},password:${JSON.stringify(process.env.LOGIN_PASSWORD)}})}).then(r=>r.status)`,
    awaitPromise: true,
  });
  console.log('LOGIN status', answer.result?.value);
}

s('Page.navigate', { url });
let loaded = false;
for (let i = 0; i < 60 && !loaded; i += 1) {
  await sleep(250);
  loaded = events.some((event) => event.method === 'Page.loadEventFired');
}
console.log('LOAD event', loaded);
await sleep(wait);

for (const selector of clicks) {
  const hit = await s('Runtime.evaluate', {
    expression: `(() => { const el = document.querySelector(${JSON.stringify(selector)}); if (!el) return 'missing'; el.click(); return 'clicked'; })()`,
    returnByValue: true,
  });
  console.log('CLICK', selector, hit.result?.value);
  await sleep(1000);
}

let ok = false;
try {
  const heap = await s('Runtime.getHeapUsage', {}, 10000);
  console.log('HEAP usedMB', Math.round((heap.usedSize || 0) / 1e6));
  const text = await s('Runtime.evaluate', {
    expression: 'document.body ? document.body.innerText.replace(/\\s+/g," ").slice(0,2000) : "(no body)"',
    returnByValue: true,
  }, 10000);
  console.log('TEXT', text.result?.value);
  const shot = await s('Page.captureScreenshot', { format: 'png' }, 15000);
  writeFileSync(out, Buffer.from(shot.data, 'base64'));
  console.log('SHOT', out);
  ok = true;
} catch (error) {
  console.log('RESULT renderer hung:', error.message);
}

const exceptions = events
  .filter((event) => event.method === 'Runtime.exceptionThrown')
  .map((event) => event.params.exceptionDetails.exception?.description || event.params.exceptionDetails.text);
const consoleErrors = events
  .filter((event) => event.method === 'Runtime.consoleAPICalled' && event.params.type === 'error')
  .map((event) => event.params.args.map((a) => a.value ?? a.description ?? '').join(' '));
const requests = events
  .filter((event) => event.method === 'Network.requestWillBeSent')
  .map((event) => event.params.request.url.replace(origin, '').split('?')[0]);
const counts = {};
for (const request of requests) counts[request] = (counts[request] || 0) + 1;
console.log('REQUESTS', requests.length, 'TOP', JSON.stringify(Object.entries(counts).sort((a, b) => b[1] - a[1]).slice(0, 8)));
console.log('EXCEPTIONS', JSON.stringify(exceptions));
console.log('CONSOLE_ERRORS', JSON.stringify(consoleErrors));

ws.close();
chrome.kill('SIGKILL');
process.exit(ok && exceptions.length === 0 ? 0 : 1);
