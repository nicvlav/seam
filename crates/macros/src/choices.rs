//! The derive behind `seam_params::Choices`, which writes an enum's labels
//! and its two mappings from the enum's own order.

use proc_macro2::{Literal, TokenStream as Tokens};
use quote::{quote, quote_spanned};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{Attribute, Data, DeriveInput, Error, Fields, LitStr, Variant};

/// The `impl Choices` for `input`, with the constants that check it, or
/// every mistake found in it.
pub(crate) fn choices(input: &DeriveInput) -> syn::Result<Tokens> {
    let name = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            name,
            "Choices is derived for an enum, not a struct or a union",
        ));
    };
    if !input.generics.params.is_empty() {
        return Err(Error::new_spanned(
            &input.generics,
            "Choices is not derived for a generic enum: write it by hand",
        ));
    }
    if data.variants.len() < 2 {
        return Err(Error::new_spanned(
            name,
            "Choices is derived for an enum of two variants or more",
        ));
    }

    let mut errors = misplaced_marks(&input.attrs);
    let mut labels = Vec::new();
    let mut checks = Vec::new();
    for (position, variant) in data.variants.iter().enumerate() {
        match label(variant) {
            Ok(label) => labels.push(label),
            Err(error) => combine(&mut errors, error),
        }
        if let Some((_, discriminant)) = &variant.discriminant {
            // A written discriminant is refused on itself unless it is the
            // position, which is the index a choice is saved as.
            let ident = &variant.ident;
            let position = Literal::usize_unsuffixed(position);
            checks.push(quote_spanned! { discriminant.span() =>
                const _: () = ::seam_params::discriminant_is_position(
                    #name::#ident as usize,
                    #position,
                );
            });
        }
    }
    if let Some(errors) = errors {
        return Err(errors);
    }

    let idents: Vec<_> = data.variants.iter().map(|variant| &variant.ident).collect();
    let positions: Vec<_> = (0..idents.len()).map(Literal::usize_unsuffixed).collect();
    let (last, earlier) = idents.split_last().expect("two variants or more");
    let earlier_positions = positions.get(..earlier.len()).unwrap_or_default();
    // The labels are refused on the enum, once, and not on each declaration
    // of a choice over it.
    let labels_checked = quote_spanned! { name.span() =>
        const _: () = ::seam_params::labels_are_valid(
            <#name as ::seam_params::Choices>::LABELS,
        );
    };
    Ok(quote! {
        #[automatically_derived]
        impl ::seam_params::Choices for #name {
            const LABELS: &'static [&'static str] = &[#(#labels),*];

            fn from_index(index: usize) -> Self {
                match index {
                    #(#earlier_positions => Self::#earlier,)*
                    _ => Self::#last,
                }
            }

            fn index(self) -> usize {
                match self {
                    #(Self::#idents => #positions,)*
                }
            }
        }

        #labels_checked
        #(#checks)*
    })
}

/// The label of `variant`: its `#[choice("...")]` mark's text, or else its
/// name.
fn label(variant: &Variant) -> syn::Result<LitStr> {
    if !matches!(variant.fields, Fields::Unit) {
        return Err(Error::new_spanned(
            &variant.fields,
            "Choices is derived for an enum whose variants have no fields",
        ));
    }
    let mut marks = variant
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("choice"));
    let Some(mark) = marks.next() else {
        let name = variant.ident.unraw();
        return Ok(LitStr::new(&name.to_string(), name.span()));
    };
    if let Some(again) = marks.next() {
        return Err(Error::new_spanned(
            again,
            "a variant has one #[choice] mark",
        ));
    }
    mark.parse_args::<LitStr>().map_err(|error| {
        Error::new(
            error.span(),
            "a #[choice] mark holds the label as one string: #[choice(\"12 dB/oct\")]",
        )
    })
}

/// An error on each `#[choice]` mark among the enum's own attributes.
fn misplaced_marks(attrs: &[Attribute]) -> Option<Error> {
    let mut errors = None;
    for mark in attrs.iter().filter(|attr| attr.path().is_ident("choice")) {
        combine(
            &mut errors,
            Error::new_spanned(mark, "a #[choice] mark goes on a variant"),
        );
    }
    errors
}

/// Adds `error` to `errors`.
fn combine(errors: &mut Option<Error>, error: Error) {
    match errors {
        Some(errors) => errors.combine(error),
        None => *errors = Some(error),
    }
}
