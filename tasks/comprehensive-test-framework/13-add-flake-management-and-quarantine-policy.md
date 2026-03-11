# 13 - Add flake management and quarantine policy

## Objective

Define how unstable coverage is handled without losing long-term signal or accountability.

## Implementation steps

- Define which test tiers block merges immediately, which tests may be quarantined temporarily, and who is responsible for restoring quarantined coverage.
- Add a quarantine list that requires an owner, root-cause issue, first-failed date, and removal criteria for each flaky test.
- Add a periodic review step that fails if quarantined tests age past the allowed limit without an owner update.
- Require every production escape or CI-found bug to add a deterministic reproducer or replay seed before the fix is considered complete.

## Deliverables

- one written quarantine policy;
- one tracked quarantine list format;
- one rule that ties escaped bugs to permanent regression coverage.
