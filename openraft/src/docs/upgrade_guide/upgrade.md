# Guide for upgrading from older version Openraft

When upgrading to a new version:
First, check the change log for the version to upgrade to.
A commit message starting with these keywords needs attention:

- DataChange: introduces breaking changes to data types. 
  A data migration tool or a compatible layer is required for the upgrade. See below.

- Change: introduces breaking changes. Your application needs adjustment to pass compilation.
  If storage related data structure changed too, a data migration tool is required for the upgrade. See below.

- Feature: introduces non-breaking new features. Your application should compile without modification.

- Improve: No breaking new features. Your application should compile without modification.

- Fix: bug fix. No modification is required.

## Upgrade from [v0.7](https://github.com/datafuselabs/openraft/tree/v0.7.4) to [v0.8](https://github.com/datafuselabs/openraft/tree/release-0.8):

[Change log v0.8](https://github.com/datafuselabs/openraft/blob/release-0.8/change-log.md)

[Guide for upgrading v0.7 to v0.8](`crate::docs::upgrade_guide::upgrade_07_08`)

## Upgrade from [v0.6.8](https://github.com/datafuselabs/openraft/tree/v0.6.8) to [v0.7.0](https://github.com/datafuselabs/openraft/tree/v0.7.0):

[Change log v0.7.0](https://github.com/datafuselabs/openraft/blob/release-0.7/change-log.md#v070)

[Guide for upgrading v0.6 to v0.7](`crate::docs::upgrade_guide::upgrade_06_07`)


## Upgrade from [v0.6.5](https://github.com/datafuselabs/openraft/tree/v0.6.5) to [v0.6.6](https://github.com/datafuselabs/openraft/tree/v0.6.6):

[Change log v0.6.6](https://github.com/datafuselabs/openraft/blob/release-0.6/change-log.md#v066)

just modify application code to pass compile.

- API changes: struct fields changed in `StorageIOError` and `Violation`.
- Data changes: none.

