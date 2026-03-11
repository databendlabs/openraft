# 11 - Wire the framework into CI with tiered quality gates

## Objective

Integrate the test layers into fast, broad, and release-grade automation without turning every PR into a long-running job.

## Implementation steps

- Split the test framework into tiers: PR-fast, main or nightly, and release-candidate.
- Put deterministic targeted suites and replay of known bad seeds in the PR-fast tier so every pull request gets fast signal.
- Put randomized campaigns, compatibility suites, and longer stress or soak jobs in scheduled or main-branch workflows with artifact retention enabled.
- Add one release gate workflow that runs the full validation matrix and blocks release publication on failure.

## Deliverables

- one documented CI tier map;
- one workflow update per tier;
- one artifact-retention policy that states what is kept and for how long.
