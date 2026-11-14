// Actions exposes the PR head as head_sha, not the synthetic merge commit
// checked out by the tests. A local reusable workflow records that immutable
// merge SHA in GitHub's referenced_workflows metadata; no artifacts or mutable
// refs/pull/*/merge lookups are needed.
const policyPaths = [
  '.github/workflows/ci.yaml', '.github/workflows/ci-reuse.yaml', 'scripts/ci-reuse.cjs',
];

function eligible(run, now) {
  return run.event === 'pull_request' && run.status === 'completed'
    && run.conclusion === 'success' && run.path === '.github/workflows/ci.yaml'
    && Date.parse(run.updated_at) >= now - 24 * 60 * 60 * 1000;
}

module.exports = async function reuseCi({ github, context, core, tree, now = Date.now() }) {
  if (context.eventName !== 'merge_group') return false;

  try {
    const { data } = await github.rest.actions.listWorkflowRuns({
      ...context.repo,
      workflow_id: 'ci.yaml',
      event: 'pull_request',
      status: 'success',
      per_page: 30,
    });

    for (const candidate of data.workflow_runs) {
      if (!eligible(candidate, now)) continue;
      // Recheck the latest attempt: a run may have been restarted since the
      // history query. Only GitHub's metadata for a successful run is evidence.
      const { data: run } = await github.rest.actions.getWorkflowRun({
        ...context.repo, run_id: candidate.id,
      });
      if (!eligible(run, now)) continue;

      const prefix = `${context.repo.owner}/${context.repo.repo}/.github/workflows/ci-reuse.yaml@`;
      const references = (run.referenced_workflows ?? []).filter(ref => ref.path === prefix + ref.sha
        && /^[0-9a-f]{40}$/.test(ref.sha));
      if (references.length !== 1) continue;
      const sha = references[0].sha;
      const { data: commit } = await github.rest.repos.getCommit({ ...context.repo, ref: sha });
      if (commit.commit.tree.sha !== tree || commit.parents.length !== 2
        || commit.parents[1].sha !== run.head_sha) continue;

      // Reject a modified caller that merely calls our reusable workflow at an
      // unrelated ref. The policy at the PR head must match the queue policy;
      // GitHub also proves the referenced merge commit has the queue's tree.
      let trusted = true;
      for (const path of policyPaths) {
        const [tested, queued] = await Promise.all([
          github.rest.repos.getContent({ ...context.repo, path, ref: run.head_sha }),
          github.rest.repos.getContent({ ...context.repo, path, ref: context.sha }),
        ]);
        if (tested.data.type !== 'file' || queued.data.type !== 'file'
          || tested.data.sha !== queued.data.sha) {
          trusted = false;
          break;
        }
      }
      if (!trusted) continue;

      core.notice(`Reusing successful PR CI: ${run.html_url} (tree ${tree}).`);
      core.setOutput('source', run.html_url);
      return true;
    }
  } catch (error) {
    // Missing metadata, permissions, or API availability must cost a test run,
    // never allow an unverified merge. A broken planner itself fails the jobs.
    core.warning(`Could not verify reusable PR CI; running tests: ${error.message}`);
  }

  return false;
};
