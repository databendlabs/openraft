Supporting utils for [Openraft](https://crates.io/crates/openraft).

`#[add_async_trait]` adds `Send` bounds to an async trait.

# Example

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