# 09 - Add end-to-end black-box scenario suites

## Objective

Exercise example applications and reference deployments through public APIs only.

## Implementation steps

- Pick the example applications and reference deployments that should serve as black-box coverage targets.
- For each target, write scenarios that use only public APIs and process boundaries, without reaching into internal test helpers.
- Cover bootstrap, adding nodes, removing nodes, failover, restore from snapshot, and rolling restart while verifying client-visible reads and writes.
- Store scenario logs and cluster state snapshots on failure using the artifact layout defined in `README.md` in this directory so breakage can be diagnosed without re-running locally first.
- Let the later CI task formalize retention rules without changing the basic layout.

## Deliverables

- one black-box suite per selected example or reference deployment;
- one shared scenario runner that exercises public APIs only;
- one artifact collection rule for failures.
