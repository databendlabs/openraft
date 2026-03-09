# Openraft Project Conventions

## Rust Style

- Always use `where` clauses for trait bounds instead of inline bounds.
  - Correct: `fn foo<T>(x: T) where T: RaftLeaderId`
  - Wrong: `fn foo<T: RaftLeaderId>(x: T)`
  - Applies to structs, impls, and trait definitions as well.
