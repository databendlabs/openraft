const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');

const fastWorkflow = fs.readFileSync(path.join(__dirname, '../../.github/workflows/ci.yaml'), 'utf8');
const workflow = fs.readFileSync(path.join(__dirname, '../../.github/workflows/ci-full.yaml'), 'utf8');
// Exercise the actual github-script body, not a separate copy of the policy.
const script = workflow.match(/          script: \|\n((?: {12}.*\n|\n)+)/)[1];
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const plan = new AsyncFunction('github', 'context', 'core', script);

async function select(eventName, runs = []) {
  const outputs = {};
  const queries = [];
  const context = {
    eventName,
    repo: { owner: 'owner', repo: 'openraft' },
    ref: 'refs/heads/main',
    sha: 'current-sha',
  };
  const github = {
    rest: {
      actions: {
        async listWorkflowRuns(query) {
          queries.push(query);
          assert.deepEqual(query, {
            ...context.repo,
            workflow_id: 'ci-full.yaml',
            branch: 'main',
            head_sha: context.sha,
            event: query.event,
            status: 'success',
            per_page: 1,
          });
          assert.ok(['schedule', 'workflow_dispatch'].includes(query.event));
          const matching = runs.filter(run => run.event === query.event
            && run.conclusion === query.status && run.head_sha === query.head_sha
            && run.head_branch === query.branch);
          return { data: { workflow_runs: matching.slice(0, query.per_page) } };
        },
      },
    },
  };
  const core = {
    setOutput: (key, value) => { outputs[key] = value; },
    notice: () => {},
  };
  await plan(github, context, core);
  return { outputs, queries };
}

const passed = {
  event: 'schedule', conclusion: 'success', head_sha: 'current-sha', head_branch: 'main',
};

test('a new main commit runs the full daily suite', async () => {
  const { outputs, queries } = await select('schedule');
  assert.deepEqual(outputs, { run: 'true' });
  assert.equal(queries.length, 2);
});

for (const event of ['schedule', 'workflow_dispatch']) {
  test(`a successful ${event} full run satisfies the daily check`, async () => {
    const { outputs } = await select('schedule', [{ ...passed, event }]);
    assert.deepEqual(outputs, { run: 'false' });
  });
}

for (const [reason, changes] of [
  ['fast push', { event: 'push' }],
  ['pull request', { event: 'pull_request' }],
  ['another commit', { head_sha: 'older-sha' }],
  ['another branch', { head_branch: 'feature' }],
  ['failed full run', { conclusion: 'failure' }],
  ['cancelled full run', { conclusion: 'cancelled' }],
]) {
  test(`${reason} cannot satisfy the daily check`, async () => {
    const { outputs } = await select('schedule', [{ ...passed, ...changes }]);
    assert.deepEqual(outputs, { run: 'true' });
  });
}

test('manual dispatch always runs the full suite, even after success', async () => {
  const { outputs, queries } = await select('workflow_dispatch', [passed]);
  assert.deepEqual(outputs, { run: 'true' });
  assert.equal(queries.length, 0);
});

test('a history API failure fails the plan rather than silently skipping coverage', async () => {
  const failure = new Error('API unavailable');
  await assert.rejects(plan({
    rest: { actions: { listWorkflowRuns: async () => { throw failure; } } },
  }, {
    eventName: 'schedule', repo: {}, ref: 'refs/heads/main', sha: 'current-sha',
  }, {}), failure);
});

test('fast checks run on PRs and main pushes and can be called by full CI', () => {
  const header = fastWorkflow.split('\njobs:\n')[0];
  assert.match(header, /^  pull_request:$/m);
  assert.match(header, /^  push:\n    branches:\n      - main$/m);
  assert.match(header, /^  workflow_call:$/m);
  assert.doesNotMatch(header, /^  (schedule|workflow_dispatch):/m);
});

test('full CI runs only daily or manually', () => {
  const header = workflow.split('\njobs:\n')[0];
  assert.match(header, /^  schedule: \[cron: '40 1 \* \* \*'\]$/m);
  assert.match(header, /^  workflow_dispatch:$/m);
  assert.doesNotMatch(header, /^  (pull_request|push|workflow_call):/m);
});

test('nested fast CI cannot share a concurrency group with its caller or main pushes', () => {
  // github.workflow is inherited from the caller. Literal prefixes avoid
  // self-deadlock; including the event separates nested checks from push CI.
  assert.match(fastWorkflow, /group: ci-fast-\$\{\{ github.ref \}\}-\$\{\{ github.event_name \}\}/);
  assert.match(workflow, /group: ci-full-\$\{\{ github.ref \}\}/);
});

function jobBlocks(source) {
  const jobs = source.split('\njobs:\n')[1];
  const starts = [...jobs.matchAll(/^  ([\w-]+):\n/gm)];
  return new Map(starts.map((match, i) => [match[1],
    jobs.slice(match.index, starts[i + 1]?.index ?? jobs.length)]));
}

const fastJobs = jobBlocks(fastWorkflow);
const fullJobs = jobBlocks(workflow);
test('required checks and deferred coverage stay in their respective workflows', () => {
  assert.deepEqual([...fastJobs.keys()].sort(), [
    'test-openraft-core-crates', 'test-crate-tests', 'stores', 'rt-tokio',
    'openraft-feature-test', 'tests-feature-test', 'rt-sim', 'tests-rt-sim',
    'detsim', 'detsim-threshold', 'lint',
  ].sort());
  assert.deepEqual([...fullJobs.keys()].sort(), [
    'plan', 'fast', 'test-release-build', 'openraft-test-bench', 'bench-minimal',
    'rt-monoio', 'rt-compio', 'wasm', 'metrics-otel', 'examples', 'jepsen-app',
  ].sort());
});

test('full CI reuses the fast workflow from the same commit', () => {
  assert.match(fullJobs.get('fast'), /^    uses: \.\/\.github\/workflows\/ci.yaml$/m);
});

for (const [name, body] of fastJobs) {
  test(`${name} runs unconditionally without the daily plan`, () => {
    assert.doesNotMatch(body, /^    (needs|if):/m);
    assert.ok(!fullJobs.has(name), 'fast job must not be duplicated in full CI');
  });
}

for (const [name, body] of fullJobs) {
  if (name === 'plan') continue;
  test(`${name} is gated by the daily history check`, () => {
    assert.match(body, /^    needs: plan$/m);
    assert.match(body, /^    if: needs.plan.outputs.run == 'true'$/m);
  });
}
