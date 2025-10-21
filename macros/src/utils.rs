use proc_macro::TokenStream;
use proc_macro2::TokenTree;

/// Check if the next two token is a doc attribute, such as `#[doc = "foo"]`.
///
/// An doc attribute is composed of a `#` token and a `Group` token with a `Bracket` delimiter:
/// ```text
/// Punct { ch: '#', },
/// Group {
///     delimiter: Bracket,
///     stream: TokenStream [
///         Ident { ident: "doc", },
///         Punct { ch: '=', },
///         Literal { kind: Str, symbol: " Doc", },
///     ],
/// },
/// ```
pub(crate) fn is_doc(curr: &TokenTree, next: &TokenTree) -> bool {
    let TokenTree::Punct(p) = curr else {
        return false;
    };

    if p.as_char() != '#' {
        return false;
    }

    let TokenTree::Group(g) = &next else {
        return false;
    };

    if g.delimiter() != proc_macro2::Delimiter::Bracket {
        return false;
    }
    let first = g.stream().into_iter().next();
    let Some(first) = first else {
        return false;
    };

    let TokenTree::Ident(i) = first else {
        return false;
    };

    i == "doc"
}

pub(crate) fn token_stream_with_error(mut item: TokenStream, e: syn::Error) -> TokenStream {
    item.extend(TokenStream::from(e.into_compile_error()));
    item
}
