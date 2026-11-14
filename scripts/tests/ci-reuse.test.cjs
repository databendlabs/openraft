const assert = require('node:assert/strict');
const test = require('node:test');
const reuseCi = require('../ci-reuse.cjs');

const tree = 'a'.repeat(40);
const testedSha = 'b'.repeat(40);
const headSha = 'c'.repeat(40);
const now = Date.parse('2026-11-14T08:00:00Z');
const reference = {
  path: `owner/openraft/.github/workflows/ci-reuse.yaml@${testedSha}`,
  sha: testedSha, ref: 'refs/pull/42/merge',
};
const run = {
  id: 42, event: 'pull_request', status: 'completed', conclusion: 'success',
  path: '.github/workflows/ci.yaml', head_sha: headSha,
  updated_at: '2026-11-14T07:00:00Z', referenced_workflows: [reference],
  html_url: 'https://github.com/owner/openraft/actions/runs/42',
};

async function decide(options = {}) {
  const calls = [];
  const outputs = {};
  const warnings = [];
  const context = {
    eventName: options.event ?? 'merge_group',
    repo: { owner: 'owner', repo: 'openraft' }, sha: 'queue-sha',
  };
  const github = {
    rest: {
      actions: {
        async listWorkflowRuns(query) {
          calls.push(['runs', query]);
          assert.deepEqual(query, {
            ...context.repo, workflow_id: 'ci.yaml', event: 'pull_request',
            status: 'success', per_page: 30,
          });
          if (options.apiError === 'runs') throw new Error('runs unavailable');
          return { data: { workflow_runs: options.runs ?? [run] } };
        },
        async getWorkflowRun(query) {
          calls.push(['attempt', query]);
          assert.equal(query.run_id, 42);
          if (options.apiError === 'attempt') throw new Error('attempt unavailable');
          return { data: options.latestRun ?? options.runs?.[0] ?? run };
        },
      },
      repos: {
        async getCommit(query) {
          calls.push(['commit', query]);
          assert.equal(query.ref, testedSha);
          if (options.apiError === 'commit') throw new Error('commit unavailable');
          return { data: { commit: { tree: { sha: options.testedTree ?? tree } },
            parents: options.parents ?? [{ sha: 'base-sha' }, { sha: headSha }] } };
        },
        async getContent(query) {
          calls.push(['policy', query]);
          assert.ok(['.github/workflows/ci.yaml', '.github/workflows/ci-reuse.yaml',
            'scripts/ci-reuse.cjs'].includes(query.path));
          assert.ok([headSha, context.sha].includes(query.ref));
          if (options.apiError === 'policy') throw new Error('policy unavailable');
          const changed = options.changedPolicy === query.path && query.ref === headSha;
          return { data: { type: options.policyType ?? 'file', sha: changed ? 'forged' : 'policy-blob' } };
        },
      },
    },
  };
  const core = {
    notice: () => {}, warning: message => warnings.push(message),
    setOutput: (key, value) => { outputs[key] = value; },
  };
  const reuse = await reuseCi({ github, context, core, tree, now });
  return { reuse, calls, outputs, warnings };
}

test('an identical tested merge tree reuses a successful PR run', async () => {
  const result = await decide();
  assert.equal(result.reuse, true);
  assert.deepEqual(result.outputs, { source: run.html_url });
  assert.equal(result.warnings.length, 0);
  assert.equal(result.calls.filter(([kind]) => kind === 'policy').length, 6);
});

for (const event of ['pull_request', 'push', 'schedule', 'workflow_dispatch']) {
  test(`${event} always runs tests, without a history query`, async () => {
    const result = await decide({ event });
    assert.equal(result.reuse, false);
    assert.deepEqual(result.calls, []);
  });
}

for (const [reason, change] of [
  ['failed', { conclusion: 'failure' }],
  ['cancelled', { conclusion: 'cancelled' }],
  ['unfinished', { status: 'in_progress' }],
  ['push', { event: 'push' }],
  ['daily workflow', { path: '.github/workflows/ci-full.yaml' }],
  ['older than 24 hours', { updated_at: '2026-11-13T07:59:59Z' }],
  ['unknown age', { updated_at: undefined }],
]) {
  test(`${reason} run cannot be reused`, async () => {
    const result = await decide({ runs: [{ ...run, ...change }] });
    assert.equal(result.reuse, false);
    assert.deepEqual(result.outputs, {});
    assert.equal(result.calls.length, 1);
  });
}

test('no successful history runs the suite', async () => {
  assert.equal((await decide({ runs: [] })).reuse, false);
});

test('a restarted latest attempt cannot reuse an earlier success', async () => {
  const result = await decide({ latestRun: { ...run, status: 'in_progress', conclusion: null } });
  assert.equal(result.reuse, false);
  assert.equal(result.calls.filter(([kind]) => kind === 'commit').length, 0);
});

for (const [reason, referenced_workflows] of [
  ['missing provenance', undefined],
  ['empty provenance', []],
  ['different repository', [{ ...reference, path: reference.path.replace('owner/', 'untrusted/') }]],
  ['different workflow', [{ ...reference, path: reference.path.replace('ci-reuse.yaml', 'other.yaml') }]],
  ['mutable ref', [{ ...reference, path: 'owner/openraft/.github/workflows/ci-reuse.yaml@main' }]],
  ['malformed SHA', [{ path: 'owner/openraft/.github/workflows/ci-reuse.yaml@invalid', sha: 'invalid' }]],
  ['ambiguous provenance', [reference, reference]],
]) {
  test(`${reason} runs the suite`, async () => {
    const result = await decide({ latestRun: { ...run, referenced_workflows } });
    assert.equal(result.reuse, false);
    assert.equal(result.calls.filter(([kind]) => kind === 'commit').length, 0);
  });
}

test('a changed merge tree runs the suite', async () => {
  assert.equal((await decide({ testedTree: 'different-tree' })).reuse, false);
});

for (const parents of [[], [{ sha: headSha }], [{ sha: 'base' }, { sha: 'another-pr' }]]) {
  test(`provenance must identify this PR merge commit: ${JSON.stringify(parents)}`, async () => {
    assert.equal((await decide({ parents })).reuse, false);
  });
}

for (const changedPolicy of ['.github/workflows/ci.yaml', '.github/workflows/ci-reuse.yaml',
  'scripts/ci-reuse.cjs']) {
  test(`changed ${changedPolicy} cannot forge reusable provenance`, async () => {
    assert.equal((await decide({ changedPolicy })).reuse, false);
  });
}

test('a missing or non-file policy cannot prove trust', async () => {
  assert.equal((await decide({ policyType: 'dir' })).reuse, false);
});

for (const apiError of ['runs', 'attempt', 'commit', 'policy']) {
  test(`${apiError} API failure falls back to running the suite`, async () => {
    const result = await decide({ apiError });
    assert.equal(result.reuse, false);
    assert.deepEqual(result.outputs, {});
    assert.equal(result.warnings.length, 1);
  });
}

test('an ineligible recent run does not hide an eligible older run', async () => {
  const result = await decide({ runs: [{ ...run, conclusion: 'failure' }, run], latestRun: run });
  assert.equal(result.reuse, true);
});
