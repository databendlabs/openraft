use proc_macro::TokenStream;

/// This macro either emits `#[async_trait::async_trait]` or `#[asnc_trait::async_trait(?Send)]`
/// based on the activated feature set.
///
/// This assumes that the `[async_trait](https://crates.io/crates/async-trait)` crate is imported
/// as `async_trait`. If the `singlethreaded` feature is enabled, `?Send` is passed to
/// `async_trait`, thereby forcing the affected asynchronous trait functions and methods to be run
/// in the same thread.
#[proc_macro_attribute]
pub fn add_async_trait(_attr: TokenStream, item: TokenStream) -> TokenStream {
    if cfg!(feature = "singlethreaded") {
        let mut output = "#[async_trait::async_trait(?Send)]".parse::<TokenStream>().unwrap();
        output.extend(item);
        output
    } else {
        let mut output = "#[async_trait::async_trait]".parse::<TokenStream>().unwrap();
        output.extend(item);
        output
    }
}
