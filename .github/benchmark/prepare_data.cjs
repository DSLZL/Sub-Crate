'use strict';

const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');

const ORIGINAL = '9cc6b5fac966993364ae33da8797f446bbf7f571';
const TAG = /^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const SHA = /^[0-9a-f]{40}$/;
const METRICS = [
  ['Pure parsing', 'µs / call', 'lower', 'Reusable parser · 48 fixtures'],
  ['Bulk conversion', 'ms / 10k nodes', 'lower', '4 output formats · 7 paired blocks'],
  ['JavaScript pipeline', 'ms / 10k input nodes', 'lower', 'Filter, deduplicate, rename · 7 paired blocks'],
  ['8-client throughput', 'requests / second', 'higher', 'Mixed fixtures · 3 measured blocks'],
  ['CPU efficiency', 'CPU ms / 10k nodes', 'lower', 'Whole backend process · 4 output formats'],
  ['Settled process RSS', 'MiB', 'lower', '1-client load · 3 measured blocks'],
];
const TARGETS = ['ClashMeta', 'Surge', 'QX', 'sing-box'];

function positive(value) {
  assert(typeof value === 'number' && Number.isFinite(value) && value > 0, 'Invalid measurement');
  return value;
}

function relativeScore({ node, rust, better }) {
  positive(node);
  positive(rust);
  assert(better === 'lower' || better === 'higher', 'Invalid metric direction');
  return positive(100 * (better === 'lower' ? node / rust : rust / node));
}

function median(values) {
  assert(values.length > 0, 'Missing measurements');
  const sorted = values.map(positive).sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function summarize(report, { sha, harness, tag }) {
  assert(report.version === 1 && report.status === 'passed', 'Failed or incompatible report');
  assert(report.machine === 'x86_64' && report.original === ORIGINAL, 'Unexpected baseline or runner');
  assert(SHA.test(sha) && report.rust === sha && SHA.test(harness) && report.harness === harness
    && TAG.test(tag) && report.release_tag === tag, 'Application or harness source mismatch');
  const cases = report.cases;
  assert(Array.isArray(cases) && cases.length === 48 && new Set(cases).size === 48
    && cases.every(id => typeof id === 'string'), 'Incomplete, duplicate, or diagnostic fixtures');
  for (const name of ['pure', 'http', 'concurrency', 'release_workloads']) assert(Array.isArray(report[name]), `Missing ${name}`);
  assert.equal(report.release_workloads.length, 70, 'Missing or extra release workloads');
  function blocks(collection, count, fields) {
    const rows = report[collection].filter(row => row && Object.entries(fields).every(([k, v]) => row[k] === v));
    assert.deepEqual(rows.map(row => row.block).sort((a, b) => a - b),
      Array.from({ length: count }, (_, i) => i), `Missing or duplicate ${collection} blocks`);
    return rows;
  }
  function parsing(kind) {
    return median(cases.map(id => median(blocks('pure', 7, {
      case: id, variant: kind === 'node' ? 'node' : 'rust_warm_parser',
    }).map(row => {
      assert(Number.isSafeInteger(row.iterations) && row.iterations > 0, 'Invalid iteration count');
      return positive(row.elapsed_ns) / row.iterations / 1000;
    }))));
  }
  function workload(variant, phase, target, cpu = false) {
    return median(blocks('release_workloads', 7, { variant, phase, target }).map(row => {
      assert(row.input_nodes === 10000 && row.output_nodes === (phase === 'javascript' ? 4000 : 10000), 'Incomplete workload');
      assert(Number.isSafeInteger(row.requests) && row.requests >= 4 && row.requests <= 256, 'Invalid workload count');
      assert(Array.isArray(row.samples_ns) && row.samples_ns.length === row.requests
        && row.samples_ns.every(value => Number.isSafeInteger(value) && value > 0), 'Missing workload samples');
      assert(Number.isSafeInteger(row.elapsed_ns) && row.elapsed_ns > 0, 'Invalid workload duration');
      assert.equal(row.elapsed_ns, row.samples_ns.reduce((sum, value) => sum + value, 0), 'Workload elapsed mismatch');
      assert(positive(row.cpu_s) >= 1, 'CPU block below resolution floor');
      for (const key of ['input_sha256', 'output_sha256']) {
        assert(Array.isArray(row[key]) && row[key].length === 2
          && row[key].every(value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value)), 'Invalid workload hashes');
        const reference = report.release_workloads.find(value => value.variant === 'node'
          && value.phase === phase && value.target === target && value.block === 0);
        assert.deepEqual(row[key], reference[key], 'Different paired workload');
      }
      assert(phase === 'javascript' ? /^[a-f0-9]{64}$/.test(row.script_sha256) : row.script_sha256 === null, 'Invalid script identity');
      if (phase === 'javascript') assert.equal(row.script_sha256,
        report.release_workloads.find(value => value.phase === phase).script_sha256, 'Different paired scripts');
      return cpu ? row.cpu_s * 1000 / row.requests : positive(row.elapsed_ns) / row.requests / 1e6;
    }));
  }
  function load(variant, clients, field, scale = 1) {
    return median(blocks('concurrency', 3, { variant, clients }).map(row => positive(row[field]) / scale));
  }
  const values = [
    [parsing('node'), parsing('rust')],
    ['node', 'rust'].map(variant => median(TARGETS.map(target => workload(variant, 'conversion', target)))),
    ['node', 'rust'].map(variant => workload(variant, 'javascript', 'JSON')),
    [load('node', 8, 'rps'), load('rust', 8, 'rps')],
    ['node', 'rust'].map(variant => median(TARGETS.map(target => workload(variant, 'conversion', target, true)))),
    [load('node', 1, 'rss_settled_bytes', 1048576), load('rust', 1, 'rss_settled_bytes', 1048576)],
  ];
  return METRICS.map(([title, unit, better, note], i) => ({ title, unit, better, note,
    node: positive(values[i][0]), rust: positive(values[i][1]) }));
}

function selectSource(eventName, event, repository, release) {
  if (eventName === 'workflow_dispatch') return { action: 'pending', ...release };
  const run = event.workflow_run;
  if (eventName !== 'workflow_run' || !run || run.conclusion !== 'success'
    || run.head_repository?.full_name !== repository || run.pull_requests?.length
    || !SHA.test(run.head_sha)) return { action: 'skip' };
  if (run.path === '.github/workflows/release.yml' && run.event === 'push'
    && run.head_sha === release.sha && run.head_branch === release.tag) {
    return { action: 'pending', ...release };
  }
  if (run.path === '.github/workflows/rust-single-node.yml' && run.event === 'workflow_dispatch'
    && run.head_branch === 'main') {
    assert(Number.isSafeInteger(run.id) && run.id > 0 && Number.isSafeInteger(run.run_attempt)
      && run.run_attempt > 0, 'Invalid source run identity');
    return { action: 'passed', ...release, harness: run.head_sha, run_id: run.id, run_attempt: run.run_attempt };
  }
  return { action: 'skip' };
}

async function resolveSource(eventName, event, repository, release, readApi) {
  let run = event.workflow_run;
  if (eventName === 'workflow_dispatch' && event.inputs?.benchmark_run) {
    const { benchmark_run: id, benchmark_attempt: attempt = '1' } = event.inputs;
    assert(/^[1-9]\d*$/.test(id) && /^[1-9]\d*$/.test(attempt), 'Invalid requested run');
    run = { id: Number(id), run_attempt: Number(attempt) };
    eventName = 'workflow_run';
  }
  if (eventName === 'workflow_run') {
    assert(Number.isSafeInteger(run?.id) && run.id > 0 && Number.isSafeInteger(run.run_attempt)
      && run.run_attempt > 0, 'Invalid triggering run');
    run = await readApi(`actions/runs/${run.id}/attempts/${run.run_attempt}`);
  }
  return selectSource(eventName, { workflow_run: run }, repository, release);
}

async function api(endpoint) {
  assert(/^[\w.-]+\/[\w.-]+$/.test(process.env.GITHUB_REPOSITORY), 'Invalid repository');
  const response = await fetch(`https://api.github.com/repos/${process.env.GITHUB_REPOSITORY}/${endpoint}`, {
    headers: { Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: 'application/vnd.github+json',
      'X-GitHub-Api-Version': '2022-11-28' }, signal: AbortSignal.timeout(30000),
  });
  assert(response.ok, `GitHub API returned ${response.status}`);
  return response.json();
}

async function latestRelease() {
  const release = await api('releases/latest');
  assert(!release.draft && !release.prerelease && TAG.test(release.tag_name), 'Expected a stable release tag');
  let { object } = await api(`git/ref/tags/${encodeURIComponent(release.tag_name)}`);
  for (let depth = 0; object.type === 'tag' && depth < 10; depth++) {
    assert(SHA.test(object.sha), 'Invalid annotated tag');
    ({ object } = await api(`git/tags/${object.sha}`));
  }
  assert(object.type === 'commit' && SHA.test(object.sha), 'Tag does not resolve to a commit');
  return { tag: release.tag_name, sha: object.sha };
}

function publicData(context, metrics = []) {
  return { schema: 2, status: context.action, release_tag: context.tag, node_revision: ORIGINAL,
    rust_revision: context.sha, harness_revision: context.action === 'passed' ? context.harness : null,
    runner_arch: 'linux-x64', run_id: context.run_id || 0, run_attempt: context.run_attempt || 0,
    generated_utc: new Date().toISOString(),
    aggregation: 'Parse: median of 48 fixture medians; bulk/CPU: median of 4 format medians over 7 blocks; JS: 7 blocks; throughput/RSS: 3 blocks.',
    disclaimer: 'Measures subscription/configuration conversion, not proxy traffic.',
    normalization: 'JS = 100 points; higher is better. Rust = 100 * JS / Rust for lower-is-better metrics; '
      + '100 * Rust / JS for higher-is-better metrics.', metrics };
}

function validatePublic(data) {
  assert.deepEqual(Object.keys(data).sort(), Object.keys(publicData({})).sort(), 'Unexpected public fields');
  assert(data.schema === 2 && ['passed', 'pending'].includes(data.status), 'Unknown chart schema/status');
  assert(TAG.test(data.release_tag) && SHA.test(data.rust_revision), 'Invalid release identity');
  assert(data.node_revision === ORIGINAL && data.runner_arch === 'linux-x64', 'Unexpected chart baseline');
  assert(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z$/.test(data.generated_utc), 'Invalid timestamp');
  for (const key of ['aggregation', 'disclaimer', 'normalization']) {
    assert(data[key] === publicData({})[key], `Unexpected ${key}`);
  }
  if (data.status === 'pending') {
    assert(data.harness_revision === null && data.run_id === 0 && data.run_attempt === 0, 'Pending provenance');
    assert.deepEqual(data.metrics, [], 'Pending chart must not contain measurements');
    return;
  }
  assert(SHA.test(data.harness_revision), 'Missing test harness source');
  assert(Number.isSafeInteger(data.run_id) && data.run_id > 0 && Number.isSafeInteger(data.run_attempt)
    && data.run_attempt > 0, 'Invalid benchmark run');
  assert(Array.isArray(data.metrics) && data.metrics.length === METRICS.length, 'Missing metrics');
  data.metrics.forEach((row, i) => {
    assert.deepEqual(Object.keys(row).sort(), ['better', 'node', 'note', 'rust', 'title', 'unit']);
    for (const [j, key] of ['title', 'unit', 'better', 'note'].entries()) assert.equal(row[key], METRICS[i][j]);
    positive(row.node);
    positive(row.rust);
  });
}

function publishable(data, release, current) {
  validatePublic(data);
  if (data.release_tag !== release.tag || data.rust_revision !== release.sha) return false;
  // A delayed release event must not replace an already completed chart with a placeholder.
  if (data.status === 'pending' && current?.status === 'passed'
    && current.release_tag === release.tag && current.rust_revision === release.sha) return false;
  if (data.status === 'passed' && current?.status === 'passed' && current.release_tag === release.tag
    && current.rust_revision === release.sha && (current.run_id > data.run_id
      || (current.run_id === data.run_id && current.run_attempt > data.run_attempt))) return false;
  return true;
}

function writeJson(file, data) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(data, null, 2)}\n`);
}
function output(values) {
  fs.appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(values).map(([k, v]) => `${k}=${v}\n`).join(''));
}
function oneMetrics(root) {
  const found = [];
  function walk(directory) {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const file = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(file);
      else if (entry.name === 'metrics.json') {
        assert(entry.isFile() && !entry.isSymbolicLink(), 'Invalid metrics file');
        found.push(file);
      }
    }
  }
  assert(fs.lstatSync(root).isDirectory() && !fs.lstatSync(root).isSymbolicLink(), 'Invalid artifact directory');
  walk(root);
  assert(found.length === 1 && fs.statSync(found[0]).size < 120000000, 'Expected one bounded metrics.json');
  return JSON.parse(fs.readFileSync(found[0], 'utf8'));
}

async function main([command, ...args]) {
  if (command === 'source') {
    const event = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, 'utf8'));
    const context = await resolveSource(process.env.GITHUB_EVENT_NAME, event,
      process.env.GITHUB_REPOSITORY, await latestRelease(), api);
    if (context.action === 'passed') {
      const { artifacts } = await api(`actions/runs/${context.run_id}/artifacts?per_page=100`);
      const name = `single-node-${context.run_id}-${context.run_attempt}`;
      if (!artifacts.some(artifact => artifact.name === name && !artifact.expired)) context.action = 'skip';
    }
    writeJson(args[0], context);
    output({ action: context.action, run_id: context.run_id || '', run_attempt: context.run_attempt || '' });
    console.log(`Chart source: ${context.action}${context.tag ? ` ${context.tag} ${context.sha}` : ''}`);
  } else if (command === 'prepare') {
    const context = JSON.parse(fs.readFileSync(args[0], 'utf8'));
    const data = publicData(context, context.action === 'passed' ? summarize(oneMetrics(args[1]), context) : []);
    validatePublic(data);
    writeJson(args[2], data);
  } else if (command === 'verify') {
    const data = JSON.parse(fs.readFileSync(path.join(args[0], 'benchmark-data.json'), 'utf8'));
    const png = fs.readFileSync(path.join(args[0], 'benchmark.png'));
    assert(png.length > 4000 && png.length < 5000000 && png.subarray(0, 8)
      .equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])), 'Invalid PNG');
    assert(png.readUInt32BE(16) === 1440 && png.readUInt32BE(20) === 850, 'Unexpected chart dimensions');
    const currentFile = 'assets/benchmark-data.json';
    const current = fs.existsSync(currentFile) ? JSON.parse(fs.readFileSync(currentFile, 'utf8')) : null;
    const ready = publishable(data, await latestRelease(), current);
    output({ ready, dispatch: ready && data.status === 'pending', tag: data.release_tag });
    console.log(ready ? `Verified chart for ${data.release_tag}` : 'Skipped stale or superseded chart');
  } else {
    throw new Error('Usage: prepare_data.cjs source|prepare|verify ...');
  }
}

module.exports = { ORIGINAL, METRICS, relativeScore, summarize, selectSource, resolveSource, publicData, validatePublic, publishable, oneMetrics };
if (require.main === module) main(process.argv.slice(2)).catch(error => { console.error(error); process.exitCode = 1; });
