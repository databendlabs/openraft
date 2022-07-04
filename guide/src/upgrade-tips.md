# Upgrade tips

First, have a look at the change log for the version to upgrade to.
A commit message starting with these keywords needs attention:

- Change: introduces breaking changes. Your application needs adjustment to pass compilation.
  If storage related data structure changed too, a data migration tool is required for the upgrade. See below.

- Feature: introduces non-breaking new features. Your application should compile without modification.

- Fix: bug fix. No modification is required.


## Upgrade guide

- To upgrade from [v0.6.5](https://github.com/datafuselabs/openraft/tree/v0.6.5) to [v0.6.6](https://github.com/datafuselabs/openraft/tree/v0.6.6):
  just modify application code to pass compile.

  [Change log v0.6.6](https://github.com/datafuselabs/openraft/blob/release-0.6/change-log.md#v066)

  - API changes: struct fields changed in `StorageIOError` and `Violation`
  - Data changes: no
  
  