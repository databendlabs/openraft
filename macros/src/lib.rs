use proc_macro::TokenStream;

/// This macro emits `#[async_trait::async_trait]` if the `singlethreaded` feature is disabled.
///
/// Starting from Rust 1.75.0, the `async_fn_in_trait` language feature is generally available
/// that allows traits to contain asynchronous methods and associated functions. However, the
/// feature has several known issues that are mainly related to Rust compiler's abilities to infer
/// `Send` bounds of associated types: [`90696``](https://github.com/rust-lang/rust/issues/90696).
///
/// Therefore, if a trait requires `Send` bounds in its associated data types, this macro
/// circumvents the compiler shortcomings by using the
/// [`async-trait`](https://crates.io/crates/async-trait) crate which boxes return
/// [`Future`](https://doc.rust-lang.org/std/future/trait.Future.html) types of all the
/// asynchronous methods and associated functions of the trait.
#[proc_macro_attribute]
pub fn add_async_trait(_attr: TokenStream, item: TokenStream) -> TokenStream {
    if cfg!(feature = "singlethreaded") {
        // `async_fn_in_trait` requires the user to explicitly specify the `Send` bound for public
        // trait methods, however the `singlethreaded` feature renders the requirement irrelevant.
        let mut output = "#[allow(async_fn_in_trait)]".parse::<TokenStream>().unwrap();
        output.extend(item);
        output
    } else {
        let mut output = "#[async_trait::async_trait]".parse::<TokenStream>().unwrap();
        output.extend(item);
        output
    }
}
