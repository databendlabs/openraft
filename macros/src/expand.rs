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
#[derive(Debug, Clone)]
enum TypeOrExpr {
    Attribute(Vec<Attribute>),
    Type(Type),
    Expr(Expr),
    Tokens(TokenStream2),
    Empty,
}

impl std::hash::Hash for TypeOrExpr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            TypeOrExpr::Attribute(attrs) => {
                std::mem::discriminant(self).hash(state);
                for attr in attrs {
                    attr.to_token_stream().to_string().hash(state);
                }
            }
            TypeOrExpr::Type(t) => {
                std::mem::discriminant(self).hash(state);
                t.to_token_stream().to_string().hash(state);
            }
            TypeOrExpr::Expr(e) => {
                std::mem::discriminant(self).hash(state);
                e.to_token_stream().to_string().hash(state);
            }
            TypeOrExpr::Tokens(ts) => {
                std::mem::discriminant(self).hash(state);
                ts.to_string().hash(state);
            }
            TypeOrExpr::Empty => {
                std::mem::discriminant(self).hash(state);
            }
        }
    }
}

impl PartialEq for TypeOrExpr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TypeOrExpr::Attribute(a1), TypeOrExpr::Attribute(a2)) => {
                a1.iter().map(|a| a.to_token_stream().to_string()).collect::<Vec<_>>()
                    == a2.iter().map(|a| a.to_token_stream().to_string()).collect::<Vec<_>>()
            }
            (TypeOrExpr::Type(t1), TypeOrExpr::Type(t2)) => {
                t1.to_token_stream().to_string() == t2.to_token_stream().to_string()
            }
            (TypeOrExpr::Expr(e1), TypeOrExpr::Expr(e2)) => {
                e1.to_token_stream().to_string() == e2.to_token_stream().to_string()
            }
            (TypeOrExpr::Tokens(ts1), TypeOrExpr::Tokens(ts2)) => ts1.to_string() == ts2.to_string(),
            (TypeOrExpr::Empty, TypeOrExpr::Empty) => true,
            _ => false,
        }
    }
}

impl Eq for TypeOrExpr {}

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
            TypeOrExpr::Tokens(ts) => tokens.extend(ts.clone()),
            TypeOrExpr::Empty => {}
        }
    }
}

impl Parse for TypeOrExpr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let res = input.call(Attribute::parse_outer);
        if let Ok(r) = res
            && !r.is_empty()
        {
            return Ok(Self::Attribute(r));
        }

        // Check if empty (next token is comma or end)
        let l = input.lookahead1();
        if l.peek(Token![,]) {
            return Ok(Self::Empty);
        }

        // Collect all tokens until comma at angle-bracket depth 0
        // This handles generics correctly: Type<A, B> won't split at the inner comma
        let mut tokens = TokenStream2::new();
        let mut angle_depth = 0i32;

        while !input.is_empty() {
            // Check if we hit a comma at depth 0 (not inside angle brackets)
            if angle_depth == 0 && input.peek(Token![,]) {
                break;
            }

            let token = input.parse::<proc_macro2::TokenTree>()?;

            // Track angle bracket depth
            if let proc_macro2::TokenTree::Punct(ref punct) = token {
                match punct.as_char() {
                    '<' => angle_depth += 1,
                    '>' => angle_depth -= 1,
                    _ => {}
                }
            }

            tokens.extend(std::iter::once(token));
        }

        if tokens.is_empty() {
            return Ok(Self::Empty);
        }

        // Try to parse collected tokens as Type first
        let type_result: Result<Type, _> = syn::parse2(tokens.clone());
        if let Ok(t) = type_result {
            return Ok(Self::Type(t));
        }

        // Try to parse as Expr
        let expr_result: Result<Expr, _> = syn::parse2(tokens.clone());
        if let Ok(e) = expr_result {
            return Ok(Self::Expr(e));
        }

        // Neither worked, return as raw tokens
        Ok(Self::Tokens(tokens))
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
