# 08 - Build compatibility and upgrade test matrices

## Objective

Verify supported version, runtime, API, snapshot, and storage interoperability paths.

## Implementation steps

- Enumerate the supported upgrade paths, legacy APIs, runtime variants, snapshot formats, and storage backends that must interoperate.
- Build compatibility tests that start a cluster on an old version or legacy path, perform writes and snapshots, then restart part or all of the cluster on the newer implementation.
- Verify that logs, snapshots, membership state, and client-visible committed data remain readable and valid after upgrade.
- Add explicit pass or fail rules for unsupported version mixes so CI distinguishes unsupported combinations from regressions.

## Deliverables

- one supported compatibility matrix;
- one mixed-version or mixed-mode scenario suite;
- one clear list of unsupported combinations and expected failures.
