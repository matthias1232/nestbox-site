// The one function every browser-opened URL goes through, exercised as the bundle
// runs it.
//
// `environmentUrl` is the piece that broke live in U03: `client.js` re-exported
// `withEnvironment` from another module without importing it, which is legal syntax
// and leaves the name unbound, so every call threw a ReferenceError and the terminal,
// the desktop and the live streams could not open. A test that calls it catches that;
// a test that only greps for the name does not.
import test from 'node:test';
import assert from 'node:assert/strict';
import { environmentUrl, setEnvironment, withEnvironment } from './client.js';

const DEFAULT = '00000000-0000-0000-0000-000000000002';

test('environmentUrl appends the active data center and keeps the query', () => {
  setEnvironment(null);
  assert.equal(environmentUrl('/api/v1/sandboxes/x/terminal?tab=t-1'), '/api/v1/sandboxes/x/terminal?tab=t-1');
  setEnvironment(DEFAULT);
  assert.equal(
    environmentUrl('/api/v1/sandboxes/x/terminal?tab=t-1'),
    `/api/v1/sandboxes/x/terminal?tab=t-1&environment=${DEFAULT}`,
  );
  assert.equal(environmentUrl('/box/x/terminal/t-1'), `/box/x/terminal/t-1?environment=${DEFAULT}`);
  setEnvironment(null);
});

test('the re-export is the same function the pure module defines', () => {
  assert.equal(typeof withEnvironment, 'function');
  assert.equal(withEnvironment('/a', DEFAULT), `/a?environment=${DEFAULT}`);
});
