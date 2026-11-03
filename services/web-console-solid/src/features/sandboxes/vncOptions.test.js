import test from 'node:test';
import assert from 'node:assert/strict';
import {
  DEFAULTS,
  KEYS,
  SCALINGS,
  STICKY,
  applyOptions,
  sanitizeOptions,
  sendCombo,
  statusText,
  withSticky,
} from './vncOptions.js';

// A stand-in for noVNC's RFB: the properties applyOptions writes and the two
// methods sendCombo calls, recording everything in order.
function fakeRfb() {
  const writes = [];
  const calls = [];
  const target = {
    writes,
    calls,
    sendKey(keysym, code, down) {
      calls.push({ keysym, code, down });
    },
  };
  for (const name of ['clipViewport', 'resizeSession', 'scaleViewport', 'viewOnly', 'qualityLevel', 'compressionLevel', 'showDotCursor']) {
    Object.defineProperty(target, name, {
      get: () => writes.filter((write) => write.name === name).at(-1)?.value,
      set: (value) => writes.push({ name, value }),
    });
  }
  return target;
}

test('the defaults are Proxmox\'s: local scaling, quality 6, shared, reconnect in 5 s', () => {
  assert.deepEqual({ ...DEFAULTS }, {
    scaling: 'local',
    viewOnly: false,
    quality: 6,
    compression: 2,
    dotCursor: false,
    shared: true,
    reconnect: true,
    reconnectDelay: 5,
  });
  assert.deepEqual([...SCALINGS], ['local', 'remote', 'none']);
});

test('stored options are completed and clamped, never trusted', () => {
  assert.deepEqual(sanitizeOptions(null), { ...DEFAULTS });
  const wild = sanitizeOptions({
    scaling: 'stretched',
    viewOnly: 'yes',
    quality: 42,
    compression: -3,
    dotCursor: 1,
    shared: 'no',
    reconnect: 0,
    reconnectDelay: 999,
  });
  assert.equal(wild.scaling, 'local');
  assert.equal(wild.viewOnly, false);
  assert.equal(wild.quality, 9);
  assert.equal(wild.compression, 0);
  assert.equal(wild.dotCursor, false);
  assert.equal(wild.shared, true);
  assert.equal(wild.reconnect, true);
  assert.equal(wild.reconnectDelay, 60);
  assert.equal(sanitizeOptions({ quality: 4.6, reconnectDelay: 0.4 }).quality, 5);
  assert.equal(sanitizeOptions({ reconnectDelay: 0.4 }).reconnectDelay, 1);
  assert.equal(sanitizeOptions({ shared: false, reconnect: false }).shared, false);
  assert.equal(sanitizeOptions({ reconnect: false }).reconnect, false);
});

test('applyOptions maps the three scalings onto noVNC, clipping before scaling', () => {
  for (const [scaling, expected] of [
    ['local', { clipViewport: false, resizeSession: false, scaleViewport: true }],
    ['remote', { clipViewport: false, resizeSession: true, scaleViewport: false }],
    ['none', { clipViewport: true, resizeSession: false, scaleViewport: false }],
  ]) {
    const rfb = fakeRfb();
    applyOptions(rfb, { ...DEFAULTS, scaling });
    assert.deepEqual(
      Object.fromEntries(rfb.writes.filter((write) => write.name in expected).map((write) => [write.name, write.value])),
      expected,
    );
    const names = rfb.writes.map((write) => write.name);
    assert.ok(names.indexOf('clipViewport') < names.indexOf('scaleViewport'), 'clipping is set before scaling');
  }
});

test('applyOptions sets view only, quality, compression and the dot cursor', () => {
  const rfb = fakeRfb();
  applyOptions(rfb, { ...DEFAULTS, viewOnly: true, quality: 3, compression: 8, dotCursor: true });
  assert.equal(rfb.viewOnly, true);
  assert.equal(rfb.qualityLevel, 3);
  assert.equal(rfb.compressionLevel, 8);
  assert.equal(rfb.showDotCursor, true);
  assert.deepEqual(applyOptions(null, { quality: 1 }), { ...DEFAULTS, quality: 1 });
});

test('the key table covers Ctrl+Alt+Del, F1 to F12, Tab, Esc and the sticky modifiers', () => {
  assert.deepEqual(KEYS.ctrlAltDel.map((entry) => entry.keysym), [0xffe3, 0xffe9, 0xffff]);
  assert.deepEqual(KEYS.ctrlAltDel.map((entry) => entry.code), ['ControlLeft', 'AltLeft', 'Delete']);
  for (let n = 1; n <= 12; n += 1) {
    assert.deepEqual(KEYS[`ctrlAltF${n}`].map((entry) => entry.keysym), [0xffe3, 0xffe9, 0xffbe + (n - 1)]);
    assert.equal(KEYS[`ctrlAltF${n}`][2].code, `F${n}`);
  }
  assert.equal(KEYS.tab[0].keysym, 0xff09);
  assert.equal(KEYS.esc[0].keysym, 0xff1b);
  assert.equal(KEYS.printScreen[0].keysym, 0xff61);
  assert.deepEqual([...STICKY], ['ctrl', 'alt', 'win', 'shift']);
  for (const name of STICKY) {
    assert.equal(KEYS[name].length, 1);
    assert.equal(typeof KEYS[name][0].code, 'string');
  }
  assert.equal(KEYS.ctrl[0].keysym, 0xffe3);
  assert.equal(KEYS.alt[0].keysym, 0xffe9);
  assert.equal(KEYS.win[0].keysym, 0xffeb);
  assert.equal(KEYS.shift[0].keysym, 0xffe1);
});

test('sendCombo presses in order and releases in reverse', () => {
  const rfb = fakeRfb();
  assert.equal(sendCombo(rfb, 'ctrlAltDel'), true);
  assert.deepEqual(rfb.calls.map((call) => [call.keysym, call.down]), [
    [0xffe3, true], [0xffe9, true], [0xffff, true],
    [0xffff, false], [0xffe9, false], [0xffe3, false],
  ]);
  assert.equal(rfb.calls[0].code, 'ControlLeft');
  assert.equal(sendCombo(rfb, KEYS.tab), true);
  assert.equal(sendCombo(rfb, 'nope'), false);
  assert.equal(sendCombo(null, 'tab'), false);
});

test('sticky modifiers go first and are not repeated inside the combo', () => {
  const combo = withSticky(['ctrl', 'nope'], 'ctrlAltDel');
  assert.deepEqual(combo.map((entry) => entry.keysym), [0xffe3, 0xffe9, 0xffff]);
  const tab = withSticky(['alt', 'win'], 'tab');
  assert.deepEqual(tab.map((entry) => entry.keysym), [0xffe9, 0xffeb, 0xff09]);
  assert.deepEqual(withSticky([], 'tab').map((entry) => entry.keysym), [0xff09]);
  assert.deepEqual(withSticky(null, 'tab').length, 1);
});

test('the status bar says what the picture is doing', () => {
  assert.equal(statusText('open', { size: { width: 1440, height: 900 }, quality: 6 }), 'Connected (1440×900, quality 6)');
  assert.equal(statusText('open', { quality: 3 }), 'Connected (quality 3)');
  assert.equal(statusText('connecting'), 'Connecting…');
  assert.equal(statusText('closed', { reason: 'the server closed the stream' }), 'Disconnected — the server closed the stream');
  assert.equal(statusText('closed'), 'Disconnected');
  assert.equal(statusText('idle'), 'Not connected');
});
