import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  MAX_TRIES,
  closeReasonText,
  nextDelay,
  retryText,
  shouldRetry,
  socketReasonText,
  statusText,
  statusTone,
} from './terminalState.js';

describe('statusText', () => {
  it('names the three states a terminal has', () => {
    assert.equal(statusText('open'), 'Connected');
    assert.equal(statusText('connecting'), 'Connecting…');
    assert.equal(statusText('closed', 'the sandbox stopped'), 'Disconnected — the sandbox stopped');
  });

  it('appends the retry detail while it is trying again', () => {
    assert.equal(
      statusText('connecting', 'next try in 2 s (try 2 of 10)'),
      'Connecting… next try in 2 s (try 2 of 10)',
    );
  });

  it('never leaves a closed terminal without a sentence', () => {
    assert.equal(statusText('closed'), 'Disconnected — the connection closed');
    assert.equal(statusText('closed', '   '), 'Disconnected — the connection closed');
    assert.equal(statusText('closed', null), 'Disconnected — the connection closed');
  });

  it('treats an unknown state as not connected rather than as open', () => {
    assert.equal(statusText('half-open', 'x'), 'Disconnected — x');
  });
});

describe('statusTone', () => {
  it('is lime when open, amber while trying, red when it stopped', () => {
    assert.equal(statusTone('open'), 'ok');
    assert.equal(statusTone('connecting'), 'warn');
    assert.equal(statusTone('closed'), 'error');
    assert.equal(statusTone(null), 'error');
  });
});

describe('nextDelay', () => {
  it('waits 1, 2, 5 and then 10 seconds', () => {
    assert.deepEqual([0, 1, 2, 3].map(nextDelay), [1000, 2000, 5000, 10000]);
  });

  it('keeps 10 s for every try after the third', () => {
    assert.equal(nextDelay(4), 10000);
    assert.equal(nextDelay(9), 10000);
  });

  it('reads a missing or silly attempt as the first one', () => {
    assert.equal(nextDelay(undefined), 1000);
    assert.equal(nextDelay(-3), 1000);
    assert.equal(nextDelay('2'), 5000);
    assert.equal(nextDelay(Number.NaN), 1000);
  });
});

describe('shouldRetry', () => {
  it('allows ten automatic tries and then stops', () => {
    assert.equal(shouldRetry(0), true);
    assert.equal(shouldRetry(MAX_TRIES - 1), true);
    assert.equal(shouldRetry(MAX_TRIES), false);
    assert.equal(shouldRetry(MAX_TRIES + 5), false);
  });

  it('treats a missing attempt as the first one', () => {
    assert.equal(shouldRetry(undefined), true);
  });
});

describe('retryText', () => {
  it('says how long the wait is and how many tries are left', () => {
    assert.equal(retryText(null, 0), 'next try in 1 s (try 1 of 10)');
    assert.equal(retryText('', 2), 'next try in 5 s (try 3 of 10)');
  });

  it('keeps the reason in front so the operator still sees why', () => {
    assert.equal(
      retryText('the server answered 404: not found', 1),
      'the server answered 404: not found · next try in 2 s (try 2 of 10)',
    );
  });
});

describe('closeReasonText', () => {
  it('turns each reason the server sends into a sentence', () => {
    assert.equal(closeReasonText({ reason: 'idle' }), 'the shell was idle, so the server detached it');
    assert.equal(closeReasonText({ reason: 'sandbox_stopped' }), 'the sandbox stopped');
  });

  it('adds the runner message and keeps an unknown reason', () => {
    assert.equal(closeReasonText({ reason: 'error', message: 'fork failed' }), 'the session ended with an error: fork failed');
    assert.equal(closeReasonText({ reason: 'from_the_future' }), 'the server closed the session (from_the_future)');
  });

  it('is null when the frame says nothing', () => {
    assert.equal(closeReasonText(null), null);
    assert.equal(closeReasonText({}), null);
  });
});

describe('socketReasonText', () => {
  it('takes the server text when there is one', () => {
    assert.equal(socketReasonText({ code: 1008, reason: 'go away' }, true), 'go away');
  });

  it('says nothing for a refused handshake, so the API can answer', () => {
    assert.equal(socketReasonText({ code: 1006 }, false), null);
    assert.equal(socketReasonText({ code: 1006 }, true), null);
  });

  it('reports a clean or coded close', () => {
    assert.equal(socketReasonText({ code: 1000 }, true), 'the server closed the connection');
    assert.equal(socketReasonText({ code: 1011 }, true), 'the connection closed (code 1011)');
    assert.equal(socketReasonText({ code: 4001 }, false), 'the connection closed (code 4001)');
  });
});
