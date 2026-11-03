import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  ALLOW_GROUP, addAllow, applyPlan, dnsSuggestionsFrom, emptySpec, entries, filesystemLists, newState,
  removeEntry, runPlan, setDns, setFilesystemLists, setLandlock, setProcess, stepsFor, suggestionsFrom,
  updateEntry, validateStep,
} from './policyWizard.js';

describe('spec mutations', () => {
  it('starts with no network access', () => {
    assert.deepEqual(emptySpec().network_policies, {});
  });

  it('adds an allow entry once', () => {
    let spec = addAllow(emptySpec(), { host: 'api.github.com', port: 443, protocol: 'https' });
    spec = addAllow(spec, { host: 'api.github.com', port: 443, protocol: 'https' });
    assert.equal(entries(spec).length, 1);
    assert.equal(spec.network_policies[ALLOW_GROUP].enforcement, 'enforced');
  });

  it('ignores an empty host and upper-cases methods', () => {
    const base = emptySpec();
    assert.equal(addAllow(base, { host: ' ' }), base);
    const spec = addAllow(emptySpec(), { host: 'a.io', methods: ['get'] });
    assert.deepEqual(entries(spec)[0].endpoint.methods, ['GET']);
  });

  it('removes by flat index and drops an empty group', () => {
    let spec = addAllow(emptySpec(), { host: 'a.io', port: 443 });
    spec = addAllow(spec, { host: 'b.io', port: 443 });
    spec = removeEntry(spec, 0);
    assert.deepEqual(entries(spec).map((e) => e.endpoint.host), ['b.io']);
    spec = removeEntry(spec, 0);
    assert.deepEqual(spec.network_policies, {});
  });

  it('updates one entry', () => {
    const spec = updateEntry(addAllow(emptySpec(), { host: 'a.io' }), 0, { port: 8443 });
    assert.equal(entries(spec)[0].endpoint.port, 8443);
  });

  it('cleans the DNS list', () => {
    const list = setDns([{ action: 'deny', pattern: ' .Ads.example ' }, { action: 'deny', pattern: '.ads.example' }, { action: 'allow', pattern: '' }, { pattern: 'a b' }]);
    assert.deepEqual(list, [{ action: 'deny', pattern: '.ads.example' }]);
  });

  it('sets and clears process fields', () => {
    let spec = setProcess(emptySpec(), { run_as_user: 'dev', binaries: '/usr/bin/git, /usr/bin/curl', max_processes: '128' });
    assert.deepEqual(spec.process.binaries, ['/usr/bin/git', '/usr/bin/curl']);
    assert.equal(spec.process.max_processes, 128);
    spec = setProcess(spec, { binaries: '', max_processes: '' });
    assert.equal('binaries' in spec.process, false);
    assert.equal('max_processes' in spec.process, false);
  });

  it('sets and removes Landlock rules', () => {
    let spec = setLandlock(emptySpec(), [{ path: '/home/nestbox', access: ['read'] }, { path: ' ' }]);
    assert.equal(spec.landlock.rules.length, 1);
    spec = setLandlock(spec, []);
    assert.equal('landlock' in spec, false);
  });

  it('rebuilds the filesystem lists and keeps other access kinds', () => {
    const base = { filesystem_policy: { paths: [{ path: '/bin', access: 'execute' }] } };
    const spec = setFilesystemLists(base, { read_only: '/etc/ssl\n', read_write: ['/workspace'] });
    assert.deepEqual(filesystemLists(spec), { read_only: ['/etc/ssl'], read_write: ['/workspace'] });
    assert.ok(spec.filesystem_policy.paths.some((p) => p.access === 'execute'));
  });
});

describe('validation and steps', () => {
  it('flags a bad port', () => {
    const spec = addAllow(emptySpec(), { host: 'a.io' });
    assert.match(validateStep('network', updateEntry(spec, 0, { port: 70000 })), /Port/);
    assert.equal(validateStep('network', spec), null);
  });
  it('flags relative and parent paths', () => {
    assert.match(validateStep('filesystem', { filesystem_policy: { paths: [{ path: 'x', access: 'read_only' }] } }), /absolute/);
    assert.match(validateStep('filesystem', { filesystem_policy: { paths: [{ path: '/a/../b', access: 'read_only' }] } }), /\.\./);
  });
  it('requires a name on review', () => {
    assert.match(validateStep('review', emptySpec(), { name: ' ' }), /name/);
    assert.equal(validateStep('review', emptySpec(), { name: 'x' }), null);
  });
  it('shows the Tools step only with a sandbox', () => {
    assert.equal(stepsFor(null).some((s) => s.id === 'tools'), false);
    assert.equal(stepsFor('s1').some((s) => s.id === 'tools'), true);
    assert.equal(stepsFor(null).at(-1).id, 'review');
  });
});

describe('the apply plan', () => {
  it('creates a new policy only', () => {
    const plan = applyPlan({ ...newState(), name: 'dev' });
    assert.equal(plan.length, 1);
    assert.equal(plan[0].call.op, 'create');
  });

  it('updates an existing one with its version', () => {
    const plan = applyPlan({ ...newState({ policy: { id: 'p1', version: 3, name: 'n', scope: 'sandbox', spec: emptySpec() } }) });
    assert.deepEqual(plan[0].call, { op: 'update', id: 'p1', body: { spec: emptySpec(), version: 3 } });
  });

  it('orders save, DNS, assign, egress, window', () => {
    const state = { ...newState({ sandboxId: 's1' }), name: 'dev', dns: [{ action: 'deny', pattern: '.ads.io' }], egressMode: 'forward_proxy', windowMinutes: 15 };
    assert.deepEqual(applyPlan(state).map((p) => p.call.op), ['create', 'dnsRule', 'assign', 'egress', 'window']);
  });

  it('runs in order, substitutes the policy id and stops at a failure', async () => {
    const state = { ...newState({ sandboxId: 's1' }), name: 'dev', dns: [{ action: 'deny', pattern: '.ads.io' }], egressMode: 'off_allow_all' };
    const seen = [];
    const ops = {
      create: async () => ({ policy: { id: 'p9' } }),
      dnsRule: async (call) => { seen.push(call.form.policy_id); },
      assign: async (call) => { seen.push(call.policyId); },
      egress: async () => { throw new Error('refused'); },
    };
    const run = await runPlan(applyPlan(state), ops);
    assert.deepEqual(seen, ['p9', 'p9']);
    assert.equal(run.failed, true);
    assert.equal(run.results.at(-1).message, 'refused');
    assert.equal(run.policyId, 'p9');
  });
});

describe('suggestions', () => {
  const items = [
    { host: 'a.io', port: 443, protocol: 'https', count: 3 },
    { host: 'a.io', port: 443, protocol: 'https', count: 2 },
    { host: 'b.io', port: 80, protocol: 'http', count: 9 },
    { host: 'ads.example', protocol: 'dns', count: 4 },
  ];
  it('groups, sorts and skips what is already allowed and DNS', () => {
    const spec = addAllow(emptySpec(), { host: 'b.io', port: 80 });
    assert.deepEqual(suggestionsFrom(items, spec).map((s) => [s.host, s.count]), [['a.io', 5]]);
  });
  it('lists DNS names without a rule', () => {
    assert.deepEqual(dnsSuggestionsFrom(items, []).map((s) => s.host), ['ads.example']);
    assert.deepEqual(dnsSuggestionsFrom(items, [{ pattern: 'ads.example' }]), []);
  });
});
