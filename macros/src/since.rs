use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse::Parser;
use syn::spanned::Spanned;

use crate::utils;

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

                        name => {
                            let msg = format!(
                                "Unknown attribute {} is specified; expected one of: `version`, `date`",
                                name,
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
    pub(crate) fn append_since_doc(self, item: TokenStream) -> Result<TokenStream, syn::Error> {
        let item = proc_macro2::TokenStream::from(item);

        // Present docs to skip, in order to append `since` at bottom of doc section.
        let mut present_docs = vec![];

        // Tokens left after present docs.
        let mut last_non_doc = vec![];

        let mut it = item.clone().into_iter();
        loop {
            let Some(curr) = it.next() else {
                break;
            };
            let Some(next) = it.next() else {
                last_non_doc.push(curr);
                break;
            };

            if utils::is_doc(&curr, &next) {
                present_docs.push(curr);
                present_docs.push(next);
            } else {
                last_non_doc.push(curr);
                last_non_doc.push(next);
                break;
            }
        }

        let since_docs_str = if present_docs.is_empty() {
            format!(r#"#[doc = " {}"]"#, self.to_doc_string())
        } else {
            // If there are already docs, insert a blank line.
            format!(r#"#[doc = ""] #[doc = " {}"]"#, self.to_doc_string())
        };
        let since_docs = proc_macro2::TokenStream::from_str(&since_docs_str).unwrap();

        let present_docs: proc_macro2::TokenStream = present_docs.into_iter().collect();
        let last_non_docs: proc_macro2::TokenStream = last_non_doc.into_iter().collect();

        // Other non doc tokens.
        let other: proc_macro2::TokenStream = it.collect();

        let tokens = quote! {
            #present_docs
            #since_docs
            #last_non_docs
            #other
        };
        Ok(tokens.into())
    }

    /// Build a `Since` from a `#[since(...)]` attribute marker.
    fn from_attr(attr: &syn::Attribute) -> Result<Since, syn::Error> {
        Self::from_tokens(attr.meta.require_list()?.tokens.clone())
    }

    /// Insert the `Since:` doc line(s) into an attribute list, after any leading doc attrs.
    ///
    /// The attribute-list counterpart of [`Self::append_since_doc`], used by the syn-based
    /// struct / enum field path.
    fn append_since_doc_attrs(&self, attrs: &mut Vec<syn::Attribute>) {
        let lead_docs = attrs.iter().take_while(|a| a.path().is_ident("doc")).count();

        // If there are already docs, insert a blank line before the `Since:` line.
        let blank = if lead_docs > 0 { quote!(#[doc = ""]) } else { quote!() };
        let line = format!(" {}", self.to_doc_string());
        let new_docs = syn::Attribute::parse_outer
            .parse2(quote!(#blank #[doc = #line]))
            .expect("since doc attributes are well-formed");

        for (offset, attr) in new_docs.into_iter().enumerate() {
            attrs.insert(lead_docs + offset, attr);
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

        chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|_e| {
            syn::Error::new(
                span,
                format!(
                    "`date`(`{}`) is not valid date string. Expected format: yyyy-mm-dd",
                    date_str
                ),
            )
        })?;

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
    match &mut input.data {
        syn::Data::Struct(data) => {
            for field in &mut data.fields {
                found |= rewrite_attrs_since(&mut field.attrs)?;
            }
        }
        syn::Data::Enum(data) => {
            for variant in &mut data.variants {
                found |= rewrite_attrs_since(&mut variant.attrs)?;
                for field in &mut variant.fields {
                    found |= rewrite_attrs_since(&mut field.attrs)?;
                }
            }
        }
        syn::Data::Union(data) => {
            for field in &mut data.fields.named {
                found |= rewrite_attrs_since(&mut field.attrs)?;
            }
        }
    }
    Ok(found)
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

/// Whether `attr` is a `#[since(...)]` marker, matching both `since` and `openraft_macros::since`.
fn is_since(attr: &syn::Attribute) -> bool {
    attr.path().segments.last().is_some_and(|s| s.ident == "since")
}
