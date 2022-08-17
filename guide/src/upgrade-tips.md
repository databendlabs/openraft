# Upgrade tips

First, have a look at the change log for the version to upgrade to.
A commit message starting with these keywords needs attention:

- Change: introduces breaking changes. Your application needs adjustment to pass compilation.
  If storage related data structure changed too, a data migration tool is required for the upgrade. See below.

- Feature: introduces non-breaking new features. Your application should compile without modification.

- Fix: bug fix. No modification is required.


## Upgrade from [v0.6.8](https://github.com/datafuselabs/openraft/tree/v0.6.8) to [v0.7.0](https://github.com/datafuselabs/openraft/tree/v0.7.0):

[Change log v0.7.0](https://github.com/datafuselabs/openraft/blob/release-0.7/change-log.md#v070)

[Guide for upgrading v0.6 to v0.7](./upgrade-v06-v07.md)


## Upgrade from [v0.6.5](https://github.com/datafuselabs/openraft/tree/v0.6.5) to [v0.6.6](https://github.com/datafuselabs/openraft/tree/v0.6.6):

[Change log v0.6.6](https://github.com/datafuselabs/openraft/blob/release-0.6/change-log.md#v066)

just modify application code to pass compile.

- API changes: struct fields changed in `StorageIOError` and `Violation`.
- Data changes: none.

