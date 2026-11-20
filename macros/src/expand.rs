use std::collections::HashSet;
use std::hash::Hash;
use std::hash::Hasher;
use std::mem::Discriminant;

use proc_macro2::Group;
use proc_macro2::Ident;
use proc_macro2::Spacing;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::Attribute;
use syn::Expr;
use syn::ExprTuple;
use syn::Token;
use syn::Type;
use syn::parenthesized;
use syn::parse::Parse;
use syn::parse::ParseStream;

/// One argument of a rendering tuple: the tokens that replace a template variable.
#[derive(Debug, Clone)]
enum Arg {
    Attribute(Vec<Attribute>),
    Type(Type),
    Expr(Expr),
    Tokens(TokenStream),
    Empty,
}

impl Arg {
    /// The identity used to deduplicate `KEYED` tuples: the variant and the argument's tokens.
    fn key(&self) -> (Discriminant<Self>, String) {
        (std::mem::discriminant(self), self.to_token_stream().to_string())
    }
}

impl Hash for Arg {
    fn hash<H>(&self, state: &mut H)
    where H: Hasher {
        self.key().hash(state)
    }
}

impl PartialEq for Arg {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

impl Eq for Arg {}

impl ToTokens for Arg {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Arg::Attribute(attrs) => {
                for a in attrs {
                    a.to_tokens(tokens)
                }
            }
            Arg::Type(t) => t.to_tokens(tokens),
            Arg::Expr(e) => e.to_tokens(tokens),
            Arg::Tokens(ts) => tokens.extend(ts.clone()),
            Arg::Empty => {}
        }
    }
}

impl Parse for Arg {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // Parse outer attributes on a fork first: a failed attempt must not consume the `#` of
        // an argument such as `#![allow(unused)]`, which the raw-token path below keeps.
        let fork = input.fork();
        if fork.call(Attribute::parse_outer).is_ok_and(|attrs| !attrs.is_empty()) {
            return Ok(Self::Attribute(input.call(Attribute::parse_outer)?));
        }

        // Check if empty (next token is comma or end)
        let l = input.lookahead1();
        if l.peek(Token![,]) {
            return Ok(Self::Empty);
        }

        // Collect all tokens until comma at angle-bracket depth 0
        // This handles generics correctly: Type<A, B> won't split at the inner comma.
        // The `>` of `->` and `=>` closes no generics, and a `>` comparison at depth 0 must
        // not hide the commas after it, so the depth never drops below zero.
        let mut tokens = TokenStream::new();
        let mut angle_depth = 0u32;
        // Whether the previous token is a `-` or `=` joined to the next one, as in `->` and `=>`.
        let mut after_joint_arrow_start = false;

        while !input.is_empty() {
            // Check if we hit a comma at depth 0 (not inside angle brackets)
            if angle_depth == 0 && input.peek(Token![,]) {
                break;
            }

            let token = input.parse::<TokenTree>()?;

            // Track angle bracket depth
            let mut arrow_start = false;
            if let TokenTree::Punct(ref punct) = token {
                match punct.as_char() {
                    '<' => angle_depth += 1,
                    '>' if !after_joint_arrow_start => angle_depth = angle_depth.saturating_sub(1),
                    '-' | '=' => arrow_start = punct.spacing() == Spacing::Joint,
                    _ => {}
                }
            }
            after_joint_arrow_start = arrow_start;

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

/// The parsed input of `expand!`: a template and the argument tuples to render it with.
pub(crate) struct Expand {
    /// The template variables, in the order their arguments appear in each tuple.
    variables: Vec<String>,

    /// The template body, without its braces.
    template: TokenStream,

    /// One argument tuple per rendering, after `KEYED` deduplication.
    arg_tuples: Vec<Vec<Arg>>,
}

impl Expand {
    pub(crate) fn render(&self) -> TokenStream {
        self.arg_tuples.iter().map(|args| self.substitute(self.template.clone(), args)).collect()
    }

    /// Replace every template variable in `tokens` with its argument from `args`, including
    /// variables inside delimited groups.
    fn substitute(&self, tokens: TokenStream, args: &[Arg]) -> TokenStream {
        let mut output = TokenStream::new();

        for t in tokens {
            match t {
                TokenTree::Ident(ident) => {
                    let ident_str = ident.to_string();

                    let variable_index = self.variables.iter().position(|x| x == &ident_str);
                    if let Some(variable_index) = variable_index {
                        output.extend(args[variable_index].to_token_stream());
                    } else {
                        output.extend([TokenTree::Ident(ident)]);
                    }
                }
                TokenTree::Group(group) => {
                    let mut substituted = Group::new(group.delimiter(), self.substitute(group.stream(), args));
                    substituted.set_span(group.span());
                    output.extend([TokenTree::Group(substituted)]);
                }
                other => output.extend([other]),
            }
        }

        output
    }
}

impl Parse for Expand {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        // KEYED, or !KEYED,
        let keyed = {
            let not_keyed = input.parse::<Token![!]>().is_ok();

            let keyed_lit = input.parse::<Ident>()?;
            if keyed_lit != "KEYED" {
                return Err(syn::Error::new_spanned(&keyed_lit, "Expected KEYED"));
            };
            !not_keyed
        };

        input.parse::<Token![,]>()?;

        // Template variables:
        // (K, V...)

        let variables_tuple = input.parse::<ExprTuple>()?;
        let mut variables = vec![];

        for expr in variables_tuple.elems.iter() {
            let Expr::Path(p) = expr else {
                return Err(syn::Error::new_spanned(expr, "Expected path"));
            };

            let segment = p.path.segments.first().ok_or_else(|| syn::Error::new_spanned(p, "Expected ident"))?;
            variables.push(segment.ident.to_string());
        }

        if variables.is_empty() {
            return Err(syn::Error::new_spanned(
                &variables_tuple,
                "Expected at least one template variable",
            ));
        }

        // Template body
        // => { ... }
        input.parse::<Token![=>]>()?;

        let brace_group = input.parse::<TokenTree>()?;
        let TokenTree::Group(tree) = brace_group else {
            return Err(syn::Error::new_spanned(brace_group, "Expected { ... }"));
        };
        let template = tree.stream();

        // List of arguments tuples for rendering the template
        // , (K1, V1...)...

        let mut arg_tuples = vec![];
        // The first arguments of the tuples kept so far, consulted only when `keyed`.
        let mut seen_keys = HashSet::new();

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
            let content;
            let _parenthesis = parenthesized!(content in input);
            let content_str = content.to_string();
            let content_span = content.span();

            let key = content.parse::<Arg>()?;
            let mut args = vec![key.clone()];

            loop {
                if content.is_empty() {
                    break;
                }

                content.parse::<Token![,]>()?;

                if content.is_empty() {
                    break;
                }

                args.push(content.parse::<Arg>()?);
            }

            if args.len() != variables.len() {
                return Err(syn::Error::new(
                    content_span,
                    format!(
                        "Expected the same number of arguments(`{}`) as template variables(`{:?}`)",
                        content_str, variables
                    ),
                ));
            }

            // Under `KEYED`, keep only the first tuple for each key.
            if keyed && !seen_keys.insert(key) {
                continue;
            }

            arg_tuples.push(args);
        }

        Ok(Expand {
            variables,
            template,
            arg_tuples,
        })
    }
}
