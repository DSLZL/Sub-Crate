'use strict';

const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { ORIGINAL, relativeScore, summarize, selectSource, resolveSource, publicData, validatePublic, publishable, oneMetrics } = require('./prepare_data.cjs');

const release = { tag: 'v0.0.1', sha: 'a'.repeat(40) };
const harness = 'c'.repeat(40);
const context = { action: 'passed', ...release, harness, run_id: 100, run_attempt: 1 };

function report() {
  const data = { version: 1, status: 'passed', machine: 'x86_64', original: ORIGINAL,
    rust: release.sha, harness, release_tag: release.tag, cases: [], pure: [], http: [], concurrency: [], release_workloads: [],
    privateInput: 'must-not-be-published' };
  for (let i = 0; i < 48; i++) {
    const id = `fixture-${i}`;
    data.cases.push(id);
    for (let block = 0; block < 7; block++) {
      for (const [variant, elapsed_ns] of [['node', 10000], ['rust_warm_parser', 5000]]) {
        data.pure.push({ case: id, variant, block, elapsed_ns: elapsed_ns * (i + 1), iterations: 10 });
      }
      for (const mode of ['new_connection', 'keep_alive']) {
        for (const variant of ['node', 'rust']) {
          data.http.push({ case: id, variant, mode, block, ns: Array(30).fill(variant === 'node' ? 2e6 : 1e6) });
        }
      }
    }
  }
  for (let block = 0; block < 3; block++) {
    for (const clients of [1, 8]) {
      for (const variant of ['node', 'rust']) {
        data.concurrency.push({ variant, clients, block, rps: variant === 'node' ? 100 : 200,
          rss_settled_bytes: (variant === 'node' ? 100 : 50) * 1048576 });
      }
    }
  }
  for (const [phase, targets] of [['conversion', ['ClashMeta', 'Surge', 'QX', 'sing-box']], ['javascript', ['JSON']]]) {
    for (const target of targets) {
      for (let block = 0; block < 7; block++) {
        for (const variant of ['node', 'rust']) {
          const ns = variant === 'node' ? 2e9 : 1e9;
          data.release_workloads.push({ phase, target, block, variant, input_nodes: 10000,
            output_nodes: phase === 'javascript' ? 4000 : 10000, requests: 4,
            cpu_s: variant === 'node' ? 2 : 1, elapsed_ns: ns * 4, samples_ns: Array(4).fill(ns),
            input_sha256: ['d'.repeat(64), 'e'.repeat(64)], output_sha256: ['f'.repeat(64), 'f'.repeat(64)],
            script_sha256: phase === 'javascript' ? 'c'.repeat(64) : null });
        }
      }
    }
  }
  return data;
}

test('aggregates measured blocks and fixture medians without copying private fields', () => {
  const data = publicData(context, summarize(report(), context));
  validatePublic(data);
  assert.deepEqual(data.metrics.map(row => [row.node, row.rust]), [[24.5, 12.25], [2000, 1000], [2000, 1000], [100, 200], [500, 250], [100, 50]]);
  assert.deepEqual(data.metrics.map(relativeScore), [200, 200, 200, 200, 200, 200]);
  assert.equal(data.rust_revision, release.sha);
  assert.equal(data.harness_revision, harness);
  assert(!JSON.stringify(data).includes('must-not-be-published'));
});

test('scores faster, lighter and higher-throughput results higher with JS fixed at 100', () => {
  for (const better of ['lower', 'higher']) {
    assert.equal(relativeScore({ node: 100, rust: 100, better }), 100);
    assert.equal(relativeScore({ node: 100, rust: 50, better }), better === 'lower' ? 200 : 50);
    assert.equal(relativeScore({ node: 100, rust: 200, better }), better === 'lower' ? 50 : 200);
    for (const invalid of [0, -1, NaN, Infinity, '50']) {
      assert.throws(() => relativeScore({ node: invalid, rust: 100, better }));
      assert.throws(() => relativeScore({ node: 100, rust: invalid, better }));
    }
  }
  assert.throws(() => relativeScore({ node: 100, rust: 50, better: 'unknown' }));
  assert.throws(() => relativeScore({ node: Number.MAX_VALUE, rust: Number.MIN_VALUE, better: 'lower' }));
});

test('rejects wrong sources, diagnostics, missing/duplicate blocks, and invalid measurements', () => {
  for (const mutate of [
    data => { data.status = 'failed'; },
    data => { data.harness = 'b'.repeat(40); },
    data => { data.rust = 'b'.repeat(40); },
    data => { data.release_tag = 'v0.0.0'; },
    data => { data.machine = 'aarch64'; },
    data => { data.original = 'b'.repeat(40); },
    data => { data.cases.pop(); },
    data => { data.cases[1] = data.cases[0]; },
    data => { data.pure.pop(); },
    data => { data.pure.push(data.pure[0]); },
    data => { data.pure[0].elapsed_ns = NaN; },
    data => { data.pure[0].iterations = true; },
    data => { data.concurrency[0].rss_settled_bytes = 0; },
    data => { data.release_workloads.pop(); },
    data => { data.release_workloads.push(data.release_workloads[0]); },
    data => { data.release_workloads[0].cpu_s = 0.5; },
    data => { data.release_workloads[0].requests = 3; },
    data => { data.release_workloads[0].output_nodes = 9999; },
    data => { data.release_workloads[0].samples_ns.pop(); },
    data => { data.release_workloads[0].elapsed_ns++; },
    data => { data.release_workloads[1].input_sha256[0] = '1'.repeat(64); },
    data => { data.release_workloads[1].output_sha256[0] = '1'.repeat(64); },
    data => { data.release_workloads.at(-1).script_sha256 = '1'.repeat(64); },
  ]) {
    const data = report();
    mutate(data);
    assert.throws(() => summarize(data, context));
  }
});

test('only successful trusted-main dispatches can supply separately verified release measurements', () => {
  const run = { id: 100, run_attempt: 1, conclusion: 'success', head_repository: { full_name: 'DSLZL/Sub-Crate' },
    head_sha: harness, head_branch: 'main', path: '.github/workflows/rust-single-node.yml',
    event: 'workflow_dispatch', pull_requests: [] };
  const select = value => selectSource('workflow_run', { workflow_run: value }, 'DSLZL/Sub-Crate', release);
  assert.deepEqual(select(run), context);
  for (const changed of [
    { conclusion: 'failure' }, { head_branch: 'feature' }, { head_branch: 'v0.0.0' }, { event: 'pull_request' },
    { head_sha: 'invalid' }, { head_repository: { full_name: 'other/fork' } }, { pull_requests: [{}] },
    { path: '.github/workflows/rust-performance.yml' },
  ]) assert.equal(select({ ...run, ...changed }).action, 'skip');
  assert.equal(select({ ...run, path: '.github/workflows/release.yml', event: 'push', head_sha: release.sha, head_branch: release.tag }).action, 'pending');
  assert.equal(selectSource('workflow_dispatch', {}, 'DSLZL/Sub-Crate', release).action, 'pending');
});

test('latest-tag checks prevent stale results, moved tags, downgrade, and older run attempts', () => {
  const complete = publicData(context, summarize(report(), context));
  const pending = publicData({ action: 'pending', ...release });
  assert(publishable(complete, release, null));
  assert(publishable(pending, release, null));
  assert(!publishable(complete, { tag: 'v0.0.2', sha: release.sha }, null));
  assert(!publishable(complete, { tag: release.tag, sha: 'b'.repeat(40) }, null));
  assert(!publishable(pending, release, complete));
  assert(!publishable(complete, release, { ...complete, run_id: 101 }));
  assert(!publishable(complete, release, { ...complete, run_attempt: 2 }));
  assert.throws(() => validatePublic({ ...complete, raw_subscription: 'not-public' }));
  assert.throws(() => validatePublic({ ...pending, metrics: complete.metrics }));
});

test('explicit completion dispatch fetches the authoritative attempt and retains the tag gate', async () => {
  const event = { inputs: { benchmark_run: '100', benchmark_attempt: '1' } };
  const run = { ...context, id: 100, conclusion: 'success', head_repository: { full_name: 'DSLZL/Sub-Crate' },
    head_sha: harness, head_branch: 'main', path: '.github/workflows/rust-single-node.yml',
    event: 'workflow_dispatch', pull_requests: [] };
  const read = async endpoint => {
    assert.equal(endpoint, 'actions/runs/100/attempts/1');
    return run;
  };
  assert.deepEqual(await resolveSource('workflow_dispatch', event, 'DSLZL/Sub-Crate', release, read), context);
  assert.equal((await resolveSource('workflow_dispatch', event, 'DSLZL/Sub-Crate', release,
    async () => ({ ...run, head_branch: 'feature' }))).action, 'skip');
  for (const id of ['-1', '0', '1e2', '100/attempts/2', '9007199254740992']) {
    await assert.rejects(resolveSource('workflow_dispatch', { inputs: { benchmark_run: id } },
      'DSLZL/Sub-Crate', release, async () => assert.fail('Invalid input reached the API')));
  }
});

test('artifact reader rejects duplicate metrics and symbolic-link substitutions', t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sub-crate-chart-input-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const nested = path.join(root, 'result');
  fs.mkdirSync(nested);
  fs.writeFileSync(path.join(nested, 'metrics.json'), '{"status":"passed"}');
  assert.deepEqual(oneMetrics(root), { status: 'passed' });
  fs.writeFileSync(path.join(root, 'metrics.json'), '{}');
  assert.throws(() => oneMetrics(root));
  fs.unlinkSync(path.join(root, 'metrics.json'));
  fs.symlinkSync(path.join(nested, 'metrics.json'), path.join(root, 'metrics.json'));
  assert.throws(() => oneMetrics(root));
});
