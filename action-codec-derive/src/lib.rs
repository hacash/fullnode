use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

/// Generate only the mechanical representation of a regular Action.
///
/// The derive intentionally does not implement `base::Action`: execution,
/// metadata, and domain capabilities remain visible in the owning crate.
#[proc_macro_derive(ActionCodec)]
pub fn derive_action_codec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_action_codec(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_action_codec(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "ActionCodec does not support generic action structs",
        ));
    }

    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "ActionCodec requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "ActionCodec can only be derived for structs",
            ));
        }
    };

    let Some(first_field) = fields.first() else {
        return Err(syn::Error::new_spanned(
            name,
            "ActionCodec requires a `kind` field",
        ));
    };
    if first_field
        .ident
        .as_ref()
        .is_none_or(|ident| ident != "kind")
    {
        return Err(syn::Error::new_spanned(
            first_field,
            "ActionCodec requires `kind` to be the first field",
        ));
    }

    let value_fields: Vec<_> = fields
        .iter()
        .filter_map(|field| {
            let ident = field.ident.as_ref()?;
            (!ident.to_string().eq("kind")).then_some(ident)
        })
        .collect();
    let value_types: Vec<_> = fields.iter().skip(1).map(|field| &field.ty).collect();
    let value_names: Vec<_> = value_fields
        .iter()
        .map(|ident| syn::LitStr::new(&ident.to_string(), ident.span()))
        .collect();

    Ok(quote! {
        impl field::Encode for #name {
            fn size(&self) -> usize {
                field::Encode::size(&self.kind)
                #( + field::Encode::size(&self.#value_fields) )*
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                field::Encode::encode_to(&self.kind, out);
                #( field::Encode::encode_to(&self.#value_fields, out); )*
            }
        }

        impl field::Decode for #name {
            fn decode(buf: &[u8]) -> sys::Ret<(Self, usize)> {
                let mut reader = field::Reader::new(buf);
                let kind: field::Uint2 = reader.read()?;
                if kind.uint() != Self::KIND {
                    return sys::normalf!(
                        "action kind mismatch: expected {} got {}",
                        Self::KIND,
                        kind.uint()
                    );
                }
                #( let #value_fields = reader.read()?; )*
                Ok((Self { kind, #( #value_fields ),* }, reader.used()))
            }
        }

        impl field::ToJSON for #name {
            fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
                let mut fields = vec![format!(
                    "\"kind\":{}",
                    field::ToJSON::to_json_fmt(&self.kind, fmt)
                )];
                #(
                    fields.push(format!(
                        "\"{}\":{}",
                        stringify!(#value_fields),
                        field::ToJSON::to_json_fmt(&self.#value_fields, fmt)
                    ));
                )*
                format!("{{{}}}", fields.join(","))
            }
        }

        impl base::ActionJsonCodec for #name {
            fn decode_json(json: &str) -> sys::Ret<Self> {
                // `kind` is implied by the action registry. Keep accepting an
                // explicit value for validation and backwards compatibility.
                let mut kind = field::Uint2::from(Self::KIND);
                #( let mut #value_fields: Option<#value_types> = None; )*
                let mut seen = std::collections::HashSet::new();

                for (key, value) in field::json_split_object(json)? {
                    if !seen.insert(key) {
                        return sys::normalf!(
                            "action {} JSON field {} is duplicated",
                            Self::KIND,
                            key
                        );
                    }
                    match key {
                        "kind" => kind = field::json_decode_value(value)?,
                        #( #value_names => #value_fields = Some(field::json_decode_value(value)?), )*
                        _ => {}
                    }
                }

                if kind.uint() != Self::KIND {
                    return sys::normalf!(
                        "action kind mismatch: expected {} got {}",
                        Self::KIND,
                        kind.uint()
                    );
                }
                #(
                    let Some(#value_fields) = #value_fields else {
                        return sys::normalf!(
                            "action {} JSON missing required field {}",
                            Self::KIND,
                            #value_names
                        );
                    };
                )*
                Ok(Self { kind, #( #value_fields ),* })
            }
        }

        impl field::FromJSON for #name {
            fn from_json(&mut self, json: &str) -> sys::Ret<()> {
                *self = <Self as base::ActionJsonCodec>::decode_json(json)?;
                Ok(())
            }
        }
    })
}
