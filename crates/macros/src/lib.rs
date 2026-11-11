//! The derive behind `seam_params::Params`, which writes a struct's
//! `declare` from a `#[param(...)]` mark on each field.
//!
//! Use it through `seam-params` with its `derive` feature, where it is
//! documented with examples: the code it writes names `::seam_params`, so it
//! cannot be used without that crate.
//!
//! The derive writes what a person would write by hand, and nothing else: one
//! `impl Params` whose `declare` adds each marked field's declaration and
//! returns the struct. Each declaration is a named constant that ends in
//! `checked`, so a plain `cargo check` refuses a key, a range or a label no
//! parameter could have, on the author's own line.

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote, quote_spanned};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Expr, Field, Fields, Ident, Token, parse_macro_input};

/// The options a declaration takes, each its `with_` method.
const OPTIONS: &[&str] = &["step", "skew_centre", "unit", "group"];

/// The flags a declaration takes, each its `as_` method.
const FLAGS: &[&str] = &["bypass"];

/// Writes `seam_params::Params` for a struct of parameters.
///
/// Documented, with examples, as `seam_params::Params`.
#[proc_macro_derive(Params, attributes(param))]
pub fn derive_params(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match params(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// The `impl Params` for `input`, or every mistake found in it.
fn params(input: &DeriveInput) -> syn::Result<Tokens> {
    let name = &input.ident;
    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(fields) => &fields.named,
            Fields::Unnamed(_) | Fields::Unit => {
                return Err(Error::new_spanned(
                    name,
                    "Params is derived for a struct with named fields",
                ));
            }
        },
        Data::Enum(_) | Data::Union(_) => {
            return Err(Error::new_spanned(
                name,
                "Params is derived for a struct, not an enum or a union",
            ));
        }
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "Params is not derived for a generic struct: write its declare by hand",
        ));
    }

    // The declarer's name, out of reach of the author's own names.
    let p = Ident::new("p", Span::mixed_site());
    let mut rows = Vec::new();
    let mut keys = Vec::new();
    let mut errors: Option<Error> = None;
    for field in fields {
        match row(field, &p, &keys) {
            Ok((row, key)) => {
                rows.push(row);
                keys.extend(key);
            }
            Err(error) => match &mut errors {
                Some(errors) => errors.combine(error),
                None => errors = Some(error),
            },
        }
    }
    if let Some(errors) = errors {
        return Err(errors);
    }
    Ok(quote! {
        #[automatically_derived]
        impl ::seam_params::Params for #name {
            fn declare(#p: &mut ::seam_params::Declarer) -> Self {
                #name { #(#rows),* }
            }
        }
    })
}

/// One field's row of the struct `declare` returns, and its key if it has a
/// mark. `earlier` holds the keys of the marked fields before it.
fn row(field: &Field, p: &Ident, earlier: &[Expr]) -> syn::Result<(Tokens, Option<Expr>)> {
    let ident = &field.ident;
    let ty = &field.ty;
    let mut marks = field
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("param"));
    let Some(mark) = marks.next() else {
        // No mark: a struct of parameters, which declares itself.
        let row = quote_spanned! { ty.span() =>
            #ident: <#ty as ::seam_params::Params>::declare(#p)
        };
        return Ok((row, None));
    };
    if let Some(again) = marks.next() {
        return Err(Error::new_spanned(again, "a field has one #[param] mark"));
    }

    let items = mark.parse_args_with(Punctuated::<Item, Token![,]>::parse_terminated)?;
    let mut arguments = Vec::new();
    let mut options = Vec::new();
    for item in items {
        match item {
            Item::Argument(argument) if options.is_empty() => arguments.push(argument),
            Item::Argument(argument) => {
                return Err(Error::new_spanned(
                    argument,
                    "a parameter's arguments come before its options",
                ));
            }
            Item::Option(name, value) => {
                known(&name, OPTIONS, "an option")?;
                let method = format_ident!("with_{}", name, span = name.span());
                options.push(quote! { .#method(#value) });
            }
            Item::Flag(word) => {
                known(&word, FLAGS, "a flag")?;
                let method = format_ident!("as_{}", word, span = word.span());
                options.push(quote! { .#method() });
            }
        }
    }

    // The field's type names the declaration, so a type alias works.
    let decl = quote_spanned! { ty.span() => <#ty as ::seam_params::Declared>::Decl };
    let checked = quote_spanned! { mark.span() => .checked() };
    let key = arguments.first().cloned();
    // A key an earlier field has is refused on this field's key.
    let fresh = key.as_ref().map(|key| {
        quote_spanned! { key.span() =>
            const _: () = ::seam_params::key_is_new(#key, &[#(#earlier),*]);
        }
    });
    let row = quote_spanned! { ty.span() =>
        #ident: {
            #fresh
            const DECLARED: #decl = <#decl>::new(#(#arguments),*) #(#options)* #checked;
            #p.add(DECLARED)
        }
    };
    Ok((row, key))
}

/// Refuses `name` unless it is one of `known`, listing them.
fn known(name: &Ident, names: &[&str], what: &str) -> syn::Result<()> {
    if names.iter().any(|known| name == known) {
        return Ok(());
    }
    let list = match names {
        [one] => format!("the one is {one}"),
        [first @ .., last] => format!("they are {} and {last}", first.join(", ")),
        [] => "there are none".to_owned(),
    };
    Err(Error::new_spanned(
        name,
        format!("`{name}` is not {what} of a parameter: {list}"),
    ))
}

/// One comma-separated item inside `#[param(...)]`.
enum Item {
    /// An argument of the declaration's `new`, as written.
    Argument(Expr),
    /// `name = value`: the declaration's `with_name(value)`.
    Option(Ident, Expr),
    /// A lone lowercase word: the declaration's `as_word()`.
    Flag(Ident),
}

impl Parse for Item {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // `name = value`, and not `name == value`, which is an argument.
        if input.peek(Ident) && input.peek2(Token![=]) && !input.peek2(Token![==]) {
            let name = input.parse()?;
            input.parse::<Token![=]>()?;
            return Ok(Item::Option(name, input.parse()?));
        }
        let ahead = input.fork();
        if let Ok(word) = ahead.parse::<Ident>()
            && (ahead.is_empty() || ahead.peek(Token![,]))
            && is_flag(&word)
        {
            input.parse::<Ident>()?;
            return Ok(Item::Flag(word));
        }
        Ok(Item::Argument(input.parse()?))
    }
}

/// A lone lowercase word is a flag. `true` and `false` are values, and a
/// constant is written in capitals.
fn is_flag(word: &Ident) -> bool {
    let text = word.to_string();
    text != "true" && text != "false" && text.starts_with(|first: char| first.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use syn::{ImplItem, Item as SynItem, Type};

    use super::*;

    /// The `with_` and `as_` methods of the declarations in `seam-params`,
    /// without their prefixes.
    fn declared_methods() -> (BTreeSet<String>, BTreeSet<String>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../params/src/declare.rs");
        let source = std::fs::read_to_string(&path).expect("seam-params' declarations");
        let file = syn::parse_file(&source).expect("declare.rs parses");
        let (mut options, mut flags) = (BTreeSet::new(), BTreeSet::new());
        let mut decls = BTreeSet::new();
        let impls = file.items.iter().filter_map(|item| match item {
            SynItem::Impl(block) if block.trait_.is_none() => Some(block),
            _ => None,
        });
        for block in impls {
            let Type::Path(ty) = &*block.self_ty else {
                continue;
            };
            let Some(name) = ty.path.segments.last().map(|s| s.ident.to_string()) else {
                continue;
            };
            if !name.ends_with("Decl") {
                continue;
            }
            decls.insert(name);
            for item in &block.items {
                let ImplItem::Fn(method) = item else { continue };
                let name = method.sig.ident.to_string();
                if let Some(option) = name.strip_prefix("with_") {
                    options.insert(option.to_owned());
                } else if let Some(flag) = name.strip_prefix("as_") {
                    flags.insert(flag.to_owned());
                }
            }
        }
        // A check that found no declaration would pass on nothing.
        assert_eq!(
            decls,
            BTreeSet::from(["BoolDecl", "ChoiceDecl", "FloatDecl"].map(String::from))
        );
        (options, flags)
    }

    #[test]
    fn the_options_and_flags_are_the_declarations_methods() {
        let (options, flags) = declared_methods();
        let listed = |names: &[&str]| names.iter().map(|&name| name.to_owned()).collect();
        assert_eq!(options, listed(OPTIONS));
        assert_eq!(flags, listed(FLAGS));
    }

    #[test]
    fn a_misspelt_option_lists_the_options() {
        let error = known(&format_ident!("stepp"), OPTIONS, "an option").unwrap_err();
        assert_eq!(
            error.to_string(),
            "`stepp` is not an option of a parameter: they are step, skew_centre, unit and group"
        );
        let error = known(&format_ident!("bypas"), FLAGS, "a flag").unwrap_err();
        assert_eq!(
            error.to_string(),
            "`bypas` is not a flag of a parameter: the one is bypass"
        );
    }

    #[test]
    fn a_mark_reads_arguments_options_and_flags() {
        let items: Punctuated<Item, Token![,]> = syn::parse_quote! {
            "gain", A == B, Pair::<f32, f32>::FIRST, true, MAX, step = 0.1, bypass
        };
        let kinds: Vec<_> = items
            .iter()
            .map(|item| match item {
                Item::Argument(_) => "argument",
                Item::Option(..) => "option",
                Item::Flag(_) => "flag",
            })
            .collect();
        let expected = [
            "argument", "argument", "argument", "argument", "argument", "option", "flag",
        ];
        assert_eq!(kinds, expected);
    }
}
