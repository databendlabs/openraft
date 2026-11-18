use proc_macro::TokenStream;

pub(crate) fn token_stream_with_error(mut item: TokenStream, e: syn::Error) -> TokenStream {
    item.extend(TokenStream::from(e.into_compile_error()));
    item
}
