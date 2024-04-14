#![doc = include_str!("lib_readme.md")]

mod since;
pub(crate) mod utils;

use proc_macro::TokenStream;
use quote::quote;
use since::Since;
use syn::parse2;
use syn::parse_macro_input;
use syn::parse_str;
use syn::token::RArrow;
use syn::Item;
use syn::ReturnType;
use syn::TraitItem;
use syn::Type;

/// This proc macro attribute optionally adds `Send` bounds to a trait.
///
/// By default, `Send` bounds will be added to the trait and to the return bounds of any async
/// functions defined withing the trait.
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
    let send_bound = parse_str("Send").unwrap();
    let default_return_type: Box<Type> = parse_str("impl std::future::Future<Output = ()> + Send").unwrap();

    match parse_macro_input!(item) {
        Item::Trait(mut input) => {
            // add `Send` bound to the trait
            input.supertraits.push(send_bound);

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
