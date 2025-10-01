use std::collections::HashSet;

use proc_macro2::Ident;
use quote::ToTokens;
use quote::quote;
use syn::__private::TokenStream2;
use syn::Attribute;
use syn::Expr;
use syn::ExprTuple;
use syn::Token;
use syn::Type;
use syn::parenthesized;
use syn::parse::Parse;
use syn::parse::ParseStream;

/// A type or an expression which is used as an argument in the `expand` macro.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum TypeOrExpr {
    Attribute(Vec<Attribute>),
    Type(Type),
    Expr(Expr),
    Empty,
}

impl ToTokens for TypeOrExpr {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            TypeOrExpr::Attribute(attrs) => {
                for a in attrs {
                    a.to_tokens(tokens)
                }
            }
            TypeOrExpr::Type(t) => t.to_tokens(tokens),
            TypeOrExpr::Expr(e) => e.to_tokens(tokens),
            TypeOrExpr::Empty => {}
        }
    }
}

impl Parse for TypeOrExpr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let res = input.call(Attribute::parse_outer);
        if let Ok(r) = res {
            if !r.is_empty() {
                return Ok(Self::Attribute(r));
            }
        }

        let res = input.parse::<Type>();
        if let Ok(t) = res {
            return Ok(Self::Type(t));
        }

        let res = input.parse::<Expr>();
        if let Ok(e) = res {
            return Ok(Self::Expr(e));
        }

        let l = input.lookahead1();
        if l.peek(Token![,]) {
            Ok(Self::Empty)
        } else {
            Err(l.error())
        }
    }
}

pub(crate) struct Expand {
    /// Whether to deduplicate by the first argument as key.
    pub(crate) keyed: bool,

    /// The template variables
    pub(crate) idents: Vec<String>,

    /// Template in tokens
    pub(crate) template: TokenStream2,

    /// Multiple arguments lists for rendering the template
    args_list: Vec<Vec<TypeOrExpr>>,

    /// The keys that have been present in one of the `args_list`.
    /// It is used for deduplication, if `keyed` is true.
    present_keys: HashSet<TypeOrExpr>,
}

impl Expand {
    pub(crate) fn render(&self) -> TokenStream2 {
        let mut output_tokens = TokenStream2::new();

        for values in self.args_list.iter() {
            for t in self.template.clone().into_iter() {
                if let proc_macro2::TokenTree::Ident(ident) = t {
                    let ident_str = ident.to_string();

                    let ident_index = self.idents.iter().position(|x| x == &ident_str);
                    if let Some(ident_index) = ident_index {
                        let replacement = &values[ident_index];
                        output_tokens.extend(replacement.to_token_stream());
                    } else {
                        output_tokens.extend(ident.into_token_stream());
                    }
                } else {
                    output_tokens.extend(t.into_token_stream());
                }
            }
        }

        quote! {
            #output_tokens
        }
    }
}

impl Parse for Expand {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut b = Expand {
            keyed: true,
            idents: vec![],
            template: Default::default(),
            args_list: vec![],
            present_keys: Default::default(),
        };

        // KEYED, or !KEYED,
        {
            let not = input.parse::<Token![!]>();
            let not_keyed = not.is_ok();

            let keyed_lit = input.parse::<Ident>()?;
            if keyed_lit != "KEYED" {
                return Err(syn::Error::new_spanned(&keyed_lit, "Expected KEYED"));
            };
            b.keyed = !not_keyed;
        }

        input.parse::<Token![,]>()?;

        // Template variables:
        // (K, V...)

        let idents_tuple = input.parse::<ExprTuple>()?;

        for expr in idents_tuple.elems.iter() {
            let Expr::Path(p) = expr else {
                return Err(syn::Error::new_spanned(expr, "Expected path"));
            };

            let segment = p.path.segments.first().ok_or_else(|| syn::Error::new_spanned(p, "Expected ident"))?;
            let ident = segment.ident.to_string();

            b.idents.push(ident);
        }

        // Template body
        // => { ... }
        {
            input.parse::<Token![=>]>()?;

            let brace_group = input.parse::<proc_macro2::TokenTree>()?;
            let proc_macro2::TokenTree::Group(tree) = brace_group else {
                return Err(syn::Error::new_spanned(brace_group, "Expected { ... }"));
            };
            b.template = tree.stream();
        }

        // List of arguments tuples for rendering the template
        // , (K1, V1...)...

        loop {
            if input.is_empty() {
                break;
            }

            input.parse::<Token![,]>()?;

            if input.is_empty() {
                break;
            }

            // A tuple of arguments for each rendering
            // (K1, V1, V2...)
            {
                let content;
                let _parenthesis = parenthesized!(content in input);
                let content_str = content.to_string();
                let content_span = content.span();

                let k = content.parse::<TypeOrExpr>()?;
                let mut args = vec![k.clone()];

                loop {
                    if content.is_empty() {
                        break;
                    }

                    content.parse::<Token![,]>()?;

                    if content.is_empty() {
                        break;
                    }

                    let v = content.parse::<TypeOrExpr>()?;
                    args.push(v);
                }

                if args.len() != b.idents.len() {
                    return Err(syn::Error::new(
                        content_span,
                        format!(
                            "Expected the same number of arguments(`{}`) as template variables(`{:?}`)",
                            content_str, b.idents
                        ),
                    ));
                }

                // Ignore duplicates if keyed
                if b.present_keys.contains(&k) && b.keyed {
                    continue;
                }

                b.present_keys.insert(k);
                b.args_list.push(args);
            }
        }

        Ok(b)
    }
}
