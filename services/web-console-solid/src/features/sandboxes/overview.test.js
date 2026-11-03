import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { agentModes, indexById, modeLabel } from './overview.js';

describe('modeLabel', () => {
  it('names every mode the list can show', () => {
    assert.equal(modeLabel('transparent'), 'Open all');
    assert.equal(modeLabel('hybrid'), 'Chats');
    assert.equal(modeLabel('forward_proxy'), 'Tunnel');
    assert.equal(modeLabel('off_allow_all'), 'OFF · all allowed');
    assert.equal(modeLabel('direct'), 'Direct');
  });

  it('shows a dash when the overview has not answered yet', () => {
    assert.equal(modeLabel(null), '—');
    assert.equal(modeLabel(undefined), '—');
  });

  it('passes an unknown mode through instead of hiding it', () => {
    assert.equal(modeLabel('from_the_future'), 'from_the_future');
  });
});

describe('agentModes', () => {
  it('lists one agent per sign-in mode, joined by a middle dot', () => {
    const agents = [
      { agent: 'claude', mode: 'subscription' },
      { agent: 'codex', mode: 'gateway' },
    ];
    assert.equal(agentModes(agents), 'Claude: subscription · Codex: gateway');
  });

  it('is empty when no entry carries agents, so the old text stays', () => {
    assert.equal(agentModes(undefined), '');
    assert.equal(agentModes([]), '');
  });

  it('keeps an agent name it does not know', () => {
    assert.equal(agentModes([{ agent: 'gemini', mode: 'gateway' }]), 'gemini: gateway');
  });
});

describe('indexById', () => {
  it('keys entries by sandbox id and skips nameless ones', () => {
    const byId = indexById([
      { sandbox_id: 'a', egress_mode: 'hybrid' },
      { egress_mode: 'direct' },
      { sandbox_id: 'b', egress_mode: 'off_allow_all' },
    ]);
    assert.deepEqual(Object.keys(byId).sort(), ['a', 'b']);
    assert.equal(byId.b.egress_mode, 'off_allow_all');
  });

  it('is an empty map when the call failed and gave nothing', () => {
    assert.deepEqual(indexById(undefined), {});
  });
});
