Supporting utils for [Openraft](https://crates.io/crates/openraft).

# `add_async_trait`

[`#[add_async_trait]`](`macro@crate::add_async_trait`) adds `Send` bounds to an async trait.

## Example

```
#[openraft_macros::add_async_trait]
trait MyTrait {
    async fn my_method(&self) -> Result<(), String>;
}
```

The above code will be transformed into:

```ignore
trait MyTrait {
    fn my_method(&self) -> impl Future<Output=Result<(), String>> + Send;
}
```


# `since`

[`#[since(version = "1.0.0")]`](`macro@crate::since`) adds a doc line `/// Since: 1.0.0`.

## Example

```rust,ignore
/// Foo function
#[since(version = "1.0.0")]
fn foo() {}
```

The above code will be transformed into:

```rust,ignore
/// Foo function
///
/// Since: 1.0.0
fn foo() {}
```


# `expand` a template

[`expand!()`](`crate::expand!`) renders a template with arguments multiple times.

# Example:

```rust
# use openraft_macros::expand;
# fn foo () {
expand!(KEYED, // ignore duplicate by `K`
        (K, T, V) => {let K: T = V;},
        (a, u64, 1),
        (a, u32, 2), // duplicate `a` will be ignored
        (c, Vec<u8>, vec![1,2])
);
# }
```

The above code will be transformed into:

```rust
# fn foo () {
let a: u64 = 1;
let c: Vec<u8> = vec![1, 2];
# }
```
