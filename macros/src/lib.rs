#![doc = include_str!("lib_readme.md")]
#![allow(clippy::uninlined_format_args)]

mod expand;
mod since;
pub(crate) mod utils;

use proc_macro::TokenStream;
use quote::quote;
use since::Since;
use syn::Item;
use syn::ReturnType;
use syn::TraitItem;
use syn::Type;
use syn::parse_macro_input;
use syn::parse_str;
use syn::parse2;
use syn::token::RArrow;

/// This proc macro attribute optionally adds `Send` bounds to a trait.
///
/// By default, `Send` bounds will be added to the trait and to the return bounds of any async
/// functions defined within the trait.
///
/// If the `singlethreaded` feature is enabled, the trait definition remains the same without any
/// added `Send` bounds.
///
/// # Example
///
/// ```
/// use openraft_macros::add_async_trait;
///
/// #[add_async_trait]
/// trait MyTrait {
///     async fn my_method(&self) -> Result<(), String>;
/// }
/// ```
///
/// The above code will be transformed into:
///
/// ```ignore
/// trait MyTrait {
///     fn my_method(&self) -> impl Future<Output=Result<(), String>> + Send;
/// }
/// ```
///
/// Note: This proc macro can only be used with traits.
///
/// # Panics
///
/// This proc macro will panic if used on anything other than trait definitions.
#[proc_macro_attribute]
pub fn add_async_trait(_attr: TokenStream, item: TokenStream) -> TokenStream {
    if cfg!(feature = "singlethreaded") {
        allow_non_send_bounds(item)
    } else {
        add_send_bounds(item)
    }
}

fn allow_non_send_bounds(item: TokenStream) -> TokenStream {
    // `async_fn_in_trait` requires the user to explicitly specify the `Send` bound for public
    // trait methods, however the `singlethreaded` feature renders the requirement irrelevant.
    let item: proc_macro2::TokenStream = item.into();
    quote! {
        #[allow(async_fn_in_trait)]
        #item
    }
    .into()
}

fn add_send_bounds(item: TokenStream) -> TokenStream {
    let default_return_type: Box<Type> = parse_str("impl std::future::Future<Output = ()> + Send").unwrap();

    match parse_macro_input!(item) {
        Item::Trait(mut input) => {
            for item in input.items.iter_mut() {
                // for each async function definition
                let TraitItem::Fn(function) = item else { continue };
                if function.sig.asyncness.is_none() {
                    continue;
                };

                // remove async from signature
                function.sig.asyncness = None;

                // wrap the return type in a `Future`
                function.sig.output = match &function.sig.output {
                    ReturnType::Default => ReturnType::Type(RArrow::default(), default_return_type.clone()),
                    ReturnType::Type(arrow, t) => {
                        let tokens = quote!(impl std::future::Future<Output = #t> + Send);
                        ReturnType::Type(*arrow, parse2(tokens).unwrap())
                    }
                };

                // if a body is defined, wrap it in an async block
                let Some(body) = &function.default else { continue };
                let body = parse2(quote!({ async move #body })).unwrap();
                function.default = Some(body);
            }

            quote!(#input).into()
        }

        _ => panic!("add_async_trait can only be used with traits"),
    }
}

/// Add a `Since` line of doc, such as `/// Since: 1.0.0`.
///
/// `#[since(version = "1.0.0")]` generates:
/// ```rust,ignore
/// /// Since: 1.0.0
/// ```
///
/// `#[since(version = "1.0.0", date = "2021-01-01")]` generates:
/// ```rust,ignore
/// /// Since: 1.0.0, Date(2021-01-01)
/// ```
///
/// - The `version` must be a valid semver string.
/// - The `date` must be a valid date string in the format `yyyy-mm-dd`.
///
/// ### Example
///
/// ```rust,ignore
/// /// Foo function
/// ///
/// /// Does something.
/// #[since(version = "1.0.0")]
/// fn foo() {}
/// ```
///
/// The above code will be transformed into:
///
/// ```rust,ignore
/// /// Foo function
/// ///
/// /// Does something.
/// ///
/// /// Since: 1.0.0
/// fn foo() {}
/// ```
#[proc_macro_attribute]
pub fn since(args: TokenStream, item: TokenStream) -> TokenStream {
    let tokens = do_since(args, item.clone());
    match tokens {
        Ok(x) => x,
        Err(e) => utils::token_stream_with_error(item, e),
    }
}

fn do_since(args: TokenStream, item: TokenStream) -> Result<TokenStream, syn::Error> {
    let since = Since::new(args)?;
    let tokens = since.append_since_doc(item)?;
    Ok(tokens)
}

/// Render a template with arguments multiple times.
///
/// The template to expand is defined as `(K,V) => { ... }`, where `K` and `V` are template
/// variables.
///
/// - The template must contain at least 1 variable.
/// - If the first macro argument is `KEYED`, the first variable serve as the key for deduplication.
///   Otherwise, the first macro argument should be `!KEYED`, and no deduplication will be
///   performed.
///
/// # Example: `KEYED` for deduplication
///
/// The following code builds a series of let statements:
/// ```
/// # use openraft_macros::expand;
/// # fn foo () {
/// expand!(
///     KEYED,
///     // Template with variables K and V, and template body, excluding the braces.
///     (K, T, V) => {let K: T = V;},
///     // Arguments for rendering the template
///     (a, u64, 1),
///     (b, String, "foo".to_string()),
///     (a, u32, 2), // duplicate a will be ignored
///     (c, Vec<u8>, vec![1,2])
/// );
/// # }
/// ```
///
/// The above code will be transformed into:
///
/// ```
/// # fn foo () {
/// let a: u64 = 1;
/// let b: String = "foo".to_string();
/// let c: Vec<u8> = vec![1, 2];
/// # }
/// ```
///
/// # Example: `!KEYED` for no deduplication
///
/// ```
/// # use openraft_macros::expand;
/// # fn foo () {
/// expand!(!KEYED, (K, T, V) => {let K: T = V;},
///                 (c, u8, 8),
///                 (c, u16, 16),
/// );
/// # }
/// ```
///
/// The above code will be transformed into:
///
/// ```
/// # fn foo () {
/// let c: u8 = 8;
/// let c: u16 = 16;
/// # }
/// ```
#[proc_macro]
pub fn expand(item: TokenStream) -> TokenStream {
    let repeat = parse_macro_input!(item as expand::Expand);
    repeat.render().into()
}
