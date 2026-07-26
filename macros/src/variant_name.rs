use proc_macro2::TokenStream;
use quote::quote;
use syn::Data;
use syn::DeriveInput;
use syn::Fields;
use syn::LitStr;
use syn::Type;
use syn::spanned::Spanned;

/// A variant of a name enum, reduced to what code generation needs.
enum Variant {
    /// A unit variant. It occupies one slot and renders as `<prefix><ident>`.
    Unit { ident: syn::Ident, text: String },

    /// A variant wrapping another name enum. It occupies that enum's `COUNT` slots and
    /// delegates its rendering to the inner enum.
    Nested { ident: syn::Ident, inner: Box<Type> },
}

/// Expand `#[derive(VariantName)]` for `input`.
pub(crate) fn expand(input: DeriveInput) -> Result<TokenStream, syn::Error> {
    let name = &input.ident;
    let prefix = parse_prefix(&input)?;
    let variants = parse_variants(&input, &prefix)?;

    // Slot offsets are not always literals: a nested variant occupies `Inner::COUNT` slots, and
    // that value is unknown here. An offset is therefore carried as a count of the unit variants
    // seen so far plus the list of inner enums seen so far.
    let mut units = 0usize;
    let mut nested = Vec::new();

    let mut all_writes = Vec::new();
    let mut index_arms = Vec::new();
    let mut as_str_arms = Vec::new();

    for variant in &variants {
        let offset = quote! { #units #( + <#nested>::COUNT )* };

        match variant {
            Variant::Unit { ident, text } => {
                all_writes.push(quote! { all[#offset] = #name::#ident; });
                index_arms.push(quote! { #name::#ident => #offset, });
                as_str_arms.push(quote! { #name::#ident => #text, });
                units += 1;
            }
            Variant::Nested { ident, inner } => {
                all_writes.push(quote! {
                    {
                        let mut i = 0usize;
                        while i < <#inner>::COUNT {
                            all[#offset + i] = #name::#ident(<#inner>::ALL[i]);
                            i += 1;
                        }
                    }
                });
                index_arms.push(quote! { #name::#ident(inner) => #offset + inner.index(), });
                as_str_arms.push(quote! { #name::#ident(inner) => inner.as_str(), });
                nested.push(inner.clone());
            }
        }
    }

    let count = quote! { #units #( + <#nested>::COUNT )* };

    // `[expr; N]` needs a value to fill the array with before the real ones are written.
    let filler = match &variants[0] {
        Variant::Unit { ident, .. } => quote! { #name::#ident },
        Variant::Nested { ident, inner } => quote! { #name::#ident(<#inner>::ALL[0]) },
    };

    Ok(quote! {
        impl #name {
            /// Total number of variants, counting a variant that wraps another name enum as
            /// that enum's `COUNT`.
            #[allow(dead_code)]
            pub const COUNT: usize = #count;

            /// All variants in canonical order, with wrapped name enums expanded in place.
            #[allow(dead_code)]
            pub const ALL: &'static [Self] = &Self::build_all();

            /// Returns the index of this variant, equal to its position in [`Self::ALL`].
            #[allow(dead_code)]
            pub const fn index(&self) -> usize {
                match self {
                    #( #index_arms )*
                }
            }

            /// Returns the string representation of this variant.
            #[allow(dead_code)]
            pub const fn as_str(&self) -> &'static str {
                match self {
                    #( #as_str_arms )*
                }
            }

            const fn build_all() -> [Self; Self::COUNT] {
                let mut all = [#filler; Self::COUNT];
                #( #all_writes )*
                all
            }
        }

        impl std::fmt::Display for #name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    })
}

/// Read the optional `#[variant_name(prefix = "...")]` attribute.
fn parse_prefix(input: &DeriveInput) -> Result<String, syn::Error> {
    let mut prefix = String::new();

    for attr in &input.attrs {
        if !attr.path().is_ident("variant_name") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("prefix") {
                prefix = meta.value()?.parse::<LitStr>()?.value();
                Ok(())
            } else {
                Err(meta.error("unsupported `variant_name` argument, expected `prefix`"))
            }
        })?;
    }

    Ok(prefix)
}

fn parse_variants(input: &DeriveInput, prefix: &str) -> Result<Vec<Variant>, syn::Error> {
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new(input.ident.span(), "VariantName only applies to enums"));
    };

    if data.variants.is_empty() {
        return Err(syn::Error::new(
            input.ident.span(),
            "VariantName needs at least one variant",
        ));
    }

    data.variants
        .iter()
        .map(|variant| {
            let ident = variant.ident.clone();

            match &variant.fields {
                Fields::Unit => Ok(Variant::Unit {
                    text: format!("{}{}", prefix, ident),
                    ident,
                }),
                Fields::Unnamed(fields) if fields.unnamed.len() == 1 => Ok(Variant::Nested {
                    ident,
                    inner: Box::new(fields.unnamed[0].ty.clone()),
                }),
                _ => Err(syn::Error::new(
                    variant.fields.span(),
                    "VariantName variants must be a unit variant or wrap exactly one name enum",
                )),
            }
        })
        .collect()
}
