use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse::ParseStream;
use syn::parse::Parser;
use syn::spanned::Spanned;

pub struct Since {
    pub(crate) version: Option<String>,
    pub(crate) date: Option<String>,
    /// The change message.
    pub(crate) change: Vec<String>,
}

impl Since {
    /// Build a `Since` struct from the given attribute arguments.
    pub(crate) fn new(args: TokenStream) -> Result<Since, syn::Error> {
        Self::from_tokens(args.into())
    }

    /// Build a `Since` struct from `proc_macro2` attribute arguments.
    ///
    /// Shared by the attribute-macro entry point and by `#[since(...)]` markers found on struct
    /// fields or enum variants.
    pub(crate) fn from_tokens(args: proc_macro2::TokenStream) -> Result<Since, syn::Error> {
        let mut since = Since {
            version: None,
            date: None,
            change: vec![],
        };

        type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;

        let parsed_args = AttributeArgs::parse_terminated.parse2(args.clone())?;

        for arg in parsed_args {
            match arg {
                syn::Meta::NameValue(namevalue) => {
                    let q = namevalue
                        .path
                        .get_ident()
                        .ok_or_else(|| syn::Error::new_spanned(&namevalue, "Must have specified ident"))?;

                    let ident = q.to_string().to_lowercase();

                    match ident.as_str() {
                        "version" => {
                            since.set_version(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        "date" => {
                            since.set_date(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        "change" => {
                            since.set_change(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        _ => {
                            let msg = format!(
                                "Unknown attribute `{}`; expected one of: `version`, `date`, `change`",
                                q,
                            );
                            return Err(syn::Error::new_spanned(namevalue, msg));
                        }
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(other, "Unknown attribute inside the macro"));
                }
            }
        }

        if since.version.is_none() {
            return Err(syn::Error::new_spanned(args, "Missing `version` attribute"));
        }

        Ok(since)
    }
    /// Append a `since` doc such as `Since: 1.0.0` to the bottom of the doc section.
    ///
    /// Only the outer attributes of `item` are parsed; the tokens after them are kept unchanged.
    pub(crate) fn append_since_doc(self, item: TokenStream) -> Result<TokenStream, syn::Error> {
        let split_outer_attrs = |input: ParseStream| {
            let attrs = input.call(syn::Attribute::parse_outer)?;
            let rest: proc_macro2::TokenStream = input.parse()?;
            Ok((attrs, rest))
        };
        let (mut attrs, rest) = split_outer_attrs.parse(item)?;

        self.append_since_doc_attrs(&mut attrs);

        Ok(quote!(#(#attrs)* #rest).into())
    }

    /// Build a `Since` from a `#[since(...)]` attribute marker.
    fn from_attr(attr: &syn::Attribute) -> Result<Since, syn::Error> {
        Self::from_tokens(attr.meta.require_list()?.tokens.clone())
    }

    /// Insert the `Since:` doc line(s) into an attribute list, after the last doc attr, or first
    /// when there is none.
    ///
    /// Inserting after the last doc attr keeps the line at the bottom of the docs even when a
    /// non-doc attribute, such as `#[allow(...)]`, comes before or between the doc comments.
    ///
    /// Used for an item-level `#[since(...)]` and for `#[since(...)]` markers on struct fields and
    /// enum variants.
    fn append_since_doc_attrs(&self, attrs: &mut Vec<syn::Attribute>) {
        let end_of_docs = attrs.iter().rposition(|a| a.path().is_ident("doc")).map_or(0, |i| i + 1);

        // If there are already docs, insert a blank line before the `Since:` line.
        let blank = if end_of_docs > 0 { quote!(#[doc = ""]) } else { quote!() };
        let line = format!(" {}", self.to_doc_string());
        let new_docs = syn::Attribute::parse_outer
            .parse2(quote!(#blank #[doc = #line]))
            .expect("since doc attributes are well-formed");

        for (offset, attr) in new_docs.into_iter().enumerate() {
            attrs.insert(end_of_docs + offset, attr);
        }
    }

    /// Build doc string: `Since: 1.0.0, Date(2021-01-01)` or  `Since: 1.0.0`
    fn to_doc_string(&self) -> String {
        let mut s = String::new();
        s.push_str("Since: ");

        if let Some(version) = &self.version {
            s.push_str(version.as_str());
        }
        if let Some(date) = &self.date {
            s.push_str(&format!(", Date({})", date));
        }

        if !self.change.is_empty() {
            s.push_str(&format!(": {}", self.change.join("; ")));
        }
        s
    }

    pub(crate) fn set_version(&mut self, ver_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        if self.version.is_some() {
            return Err(syn::Error::new(span, "`version` set multiple times."));
        }

        let ver = Self::parse_str(ver_lit, "version", span)?;

        semver::Version::parse(&ver)
            .map_err(|_e| syn::Error::new(span, format!("`version`(`{}`) is not valid semver.", ver)))?;

        self.version = Some(ver);

        Ok(())
    }

    pub(crate) fn set_date(&mut self, date_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        if self.date.is_some() {
            return Err(syn::Error::new(span, "`date` set multiple times."));
        }

        let date_str = Self::parse_str(date_lit, "date", span)?;

        if !is_valid_date(&date_str) {
            return Err(syn::Error::new(
                span,
                format!(
                    "`date`(`{}`) is not valid date string. Expected format: yyyy-mm-dd",
                    date_str
                ),
            ));
        }

        self.date = Some(date_str);

        Ok(())
    }

    pub(crate) fn set_change(&mut self, change_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        let change_str = Self::parse_str(change_lit, "change", span)?;

        self.change.push(change_str);

        Ok(())
    }

    /// Extract string from `foo` or `"foo"`
    fn parse_str(expr: syn::Expr, field: &str, span: Span) -> Result<String, syn::Error> {
        let s = match expr {
            syn::Expr::Lit(s) => match s.lit {
                syn::Lit::Str(s) => s.value(),
                syn::Lit::Verbatim(s) => s.to_string(),
                _ => {
                    return Err(syn::Error::new(
                        span,
                        format!("Failed to parse value of `{}` as string.", field),
                    ));
                }
            },
            syn::Expr::Verbatim(s) => s.to_string(),
            _ => {
                return Err(syn::Error::new(
                    span,
                    format!("Failed to parse value of `{}` as string.", field),
                ));
            }
        };
        Ok(s)
    }
}

/// Expand `#[since]` on a `struct`/`enum`/`union` that carries `#[since(...)]` markers on its
/// fields or variants.
///
/// Rust forbids attribute macros directly on fields, so a field-level `#[since]` is reachable only
/// when the enclosing container is annotated with `#[since]`: that invocation expands first and
/// rewrites the markers here. Returns `Ok(None)` when `item` is not such a container, so the caller
/// falls back to the plain item-level expansion.
pub(crate) fn try_expand_container(args: TokenStream, item: TokenStream) -> Result<Option<TokenStream>, syn::Error> {
    let Ok(mut input) = syn::parse2::<syn::DeriveInput>(item.into()) else {
        return Ok(None);
    };

    if !rewrite_field_since(&mut input)? {
        return Ok(None);
    }

    // Args on the container document the container itself; empty args only enable field rewriting.
    let args = proc_macro2::TokenStream::from(args);
    if !args.is_empty() {
        Since::from_tokens(args)?.append_since_doc_attrs(&mut input.attrs);
    }

    Ok(Some(quote!(#input).into()))
}

/// Rewrite every `#[since(...)]` marker on the fields / variants of `input` into a `Since:` doc.
///
/// Returns whether any marker was found.
fn rewrite_field_since(input: &mut syn::DeriveInput) -> Result<bool, syn::Error> {
    let mut found = false;
    for attrs in marker_sites(input) {
        found |= rewrite_attrs_since(attrs)?;
    }
    Ok(found)
}

/// Remove every `#[since(...)]` marker from the fields / variants of `item`.
///
/// When a container fails to expand, the caller re-emits the item with the error; without this,
/// rustc would also reject each marker left on a field. An item without markers, including one
/// that is not a struct, enum or union, is returned unchanged.
pub(crate) fn strip_field_markers(item: TokenStream) -> TokenStream {
    let Ok(mut input) = syn::parse2::<syn::DeriveInput>(item.clone().into()) else {
        return item;
    };

    let mut removed = false;
    for attrs in marker_sites(&mut input) {
        let before = attrs.len();
        attrs.retain(|a| !is_since(a));
        removed |= attrs.len() != before;
    }

    if !removed {
        return item;
    }
    quote!(#input).into()
}

/// The attribute lists of every field and variant of `input`, where `#[since(...)]` markers sit,
/// in source order.
fn marker_sites(input: &mut syn::DeriveInput) -> Vec<&mut Vec<syn::Attribute>> {
    match &mut input.data {
        syn::Data::Struct(data) => data.fields.iter_mut().map(|field| &mut field.attrs).collect(),
        syn::Data::Enum(data) => data
            .variants
            .iter_mut()
            .flat_map(|variant| {
                std::iter::once(&mut variant.attrs).chain(variant.fields.iter_mut().map(|field| &mut field.attrs))
            })
            .collect(),
        syn::Data::Union(data) => data.fields.named.iter_mut().map(|field| &mut field.attrs).collect(),
    }
}

/// Replace each `#[since(...)]` attribute in `attrs` with its `Since:` doc, preserving order.
fn rewrite_attrs_since(attrs: &mut Vec<syn::Attribute>) -> Result<bool, syn::Error> {
    let sinces = attrs.iter().filter(|a| is_since(a)).map(Since::from_attr).collect::<Result<Vec<_>, _>>()?;

    if sinces.is_empty() {
        return Ok(false);
    }

    attrs.retain(|a| !is_since(a));
    for since in &sinces {
        since.append_since_doc_attrs(attrs);
    }

    Ok(true)
}

/// The paths a `#[since(...)]` marker on a field or variant may use: the bare name, this crate,
/// and its re-exports in `openraft`.
const MARKER_PATHS: [&[&str]; 4] = [
    &["since"],
    &["openraft_macros", "since"],
    &["openraft", "macros", "since"],
    &["openraft", "openraft_macros", "since"],
];

/// Whether `attr` is a `#[since(...)]` marker: its path is exactly one of [`MARKER_PATHS`].
///
/// Any other attribute stays on the field for rustc, so another tool's `#[clippy::since]` is kept
/// and a misspelled path still fails to resolve.
fn is_since(attr: &syn::Attribute) -> bool {
    let path = attr.path();
    path.leading_colon.is_none()
        && MARKER_PATHS.iter().any(|names| {
            names.len() == path.segments.len()
                && names.iter().zip(&path.segments).all(|(name, segment)| segment.ident == *name)
        })
}

/// Whether `s` is an existing calendar date written as `yyyy-mm-dd`.
fn is_valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }

    let (Some(year), Some(month), Some(day)) =
        (parse_digits(&b[0..4]), parse_digits(&b[5..7]), parse_digits(&b[8..10]))
    else {
        return false;
    };

    let is_leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap => 29,
        2 => 28,
        _ => return false,
    };

    (1..=days_in_month).contains(&day)
}

/// Parse a string of ASCII digits as a decimal number; `None` if any byte is not a digit.
fn parse_digits(b: &[u8]) -> Option<u32> {
    b.iter().try_fold(0u32, |acc, c| {
        c.is_ascii_digit().then(|| acc * 10 + u32::from(c - b'0'))
    })
}

#[cfg(test)]
mod tests {
    use quote::quote;
    use syn::parse_quote;

    use super::is_since;
    use super::is_valid_date;

    #[test]
    fn test_is_since() {
        let markers: [syn::Attribute; 4] = [
            parse_quote!(#[since(version = "1.0.0")]),
            parse_quote!(#[openraft_macros::since(version = "1.0.0")]),
            parse_quote!(#[openraft::macros::since(version = "1.0.0")]),
            parse_quote!(#[openraft::openraft_macros::since(version = "1.0.0")]),
        ];
        for attr in &markers {
            assert!(is_since(attr), "{}", quote!(#attr));
        }

        // Another tool's attribute, a misspelled path and other spellings are left for rustc.
        let others: [syn::Attribute; 6] = [
            parse_quote!(#[clippy::since]),
            parse_quote!(#[openraft_macro::since(version = "1.0.0")]),
            parse_quote!(#[macros::since(version = "1.0.0")]),
            parse_quote!(#[::openraft_macros::since(version = "1.0.0")]),
            parse_quote!(#[since::since(version = "1.0.0")]),
            parse_quote!(#[doc = "since"]),
        ];
        for attr in &others {
            assert!(!is_since(attr), "{}", quote!(#attr));
        }
    }

    #[test]
    fn test_is_valid_date() {
        assert!(is_valid_date("2021-01-01"));
        assert!(is_valid_date("2021-12-31"));
        assert!(is_valid_date("2024-02-29"));
        assert!(is_valid_date("2000-02-29"));

        assert!(!is_valid_date("2021-01--"));
        assert!(!is_valid_date("2021-1-01"));
        assert!(!is_valid_date("2021-01-1"));
        assert!(!is_valid_date("2021/01/01"));
        assert!(!is_valid_date("2021-00-01"));
        assert!(!is_valid_date("2021-13-01"));
        assert!(!is_valid_date("2021-01-00"));
        assert!(!is_valid_date("2021-04-31"));
        assert!(!is_valid_date("2023-02-29"));
        assert!(!is_valid_date("1900-02-29"));
        assert!(!is_valid_date("+021-01-01"));
        assert!(!is_valid_date("2021-01-01 "));
        assert!(!is_valid_date(""));
    }
}
