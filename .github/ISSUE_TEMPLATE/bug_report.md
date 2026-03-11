---
name: Bug report
about: Create a report to help us improve
title: ''
labels: ''
assignees: ''

---

Use the priority rubric in [`CONTRIBUTING.md`](https://github.com/databendlabs/openraft/blob/main/CONTRIBUTING.md#issue-triage) when filling this report.

**Describe the bug**
A clear and concise description of what the bug is.

**Priority suggestion**
- [ ] P0 - critical: correctness, safety, data loss, security, or release blocker
- [ ] P1 - high: blocks a common workflow and has no practical workaround
- [ ] P2 - normal: limited scope or there is a reasonable workaround
- [ ] P3 - low: edge case, cleanup, or other nice-to-have fix

**To Reproduce**
Steps to reproduce the behavior:
1. Go to '...'
2. 'cargo ...'

**Expected behavior**
A clear and concise description of what you expected to happen.

**Actual behavior**
 applicable, add screenshots to help explain your problem.

**Env (please complete the following information):**
- Openraft version [e.g., a tag such as `v0.7.2` or a commit hash]
- Does the bug still there in the latest version?(`main` for the latest release branch, or `release-0.7` for 0.7 branch)?
- Rust-toolchain: [e.g. `cargo 1.65.0-nightly (ce40690a5 2022-08-09)`; Find toolchain with `cargo version` in a crate dir]
- OS: [e.g. mac os 12.0.1, centos 7.0]
- CPU: [e.g. mac m1, x86_64]

**Tracking criteria**
- Affected branch/version:
- Does it block a release or upgrade? [yes/no]
- Is there a workaround? [yes/no; if yes, describe it]

**Additional files:**
- Logs: Attach logs generate by unittest(e.g., `./openraft/_log/*`), or logs output by examples(`examples/raft-kv-*/test-cluster.sh` will print logs in `n1.log`, `n2.log`, `n3.log`)
- Cargo.lock: [e.g. `./openraft/Cargo.lock`, `./examples/raft-kv-memstore/Cargo.lock`]

**Additional context**
Add any other context about the problem here.
