use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Data, DeriveInput, Fields, Ident, Type, parse::ParseStream, parse_macro_input, spanned::Spanned,
};

/// Generates mechanical codecs for a named-field struct: Encode/Decode,
/// ToJSON/FromJSON, Default, FieldWireShape, StructSchemaProvider, and
/// WireElementName. Field types without `field::FieldWireShape` fail to
/// compile (no silent fallback).
///
/// `#[field_codec(json_only)]` keeps handwritten Encode/Decode/Default and
/// emits JSON plus schema. `optional FIELD when METHOD` and `check = path`
/// match `impl_struct_json!`. `schema = false` skips the schema trio when the
/// type already has a leaf `FieldWireShape` (e.g. AssetAmt).
#[proc_macro_derive(FieldCodec, attributes(field_codec))]
pub fn derive_field_codec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_field_codec(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[derive(Clone)]
struct FieldCodecArgs {
    json_only: bool,
    generate_schema: bool,
    optional: Option<(Ident, Ident)>,
    check: Option<syn::Path>,
}

impl Default for FieldCodecArgs {
    fn default() -> Self {
        Self {
            json_only: false,
            generate_schema: true,
            optional: None,
            check: None,
        }
    }
}

struct NamedField {
    ident: Ident,
    ty: Type,
    wire_override: Option<proc_macro2::TokenStream>,
}

fn parse_field_codec_args(input: &DeriveInput) -> syn::Result<FieldCodecArgs> {
    let mut args = FieldCodecArgs::default();
    for attr in &input.attrs {
        if !attr.path().is_ident("field_codec") {
            continue;
        }
        attr.parse_args_with(|input: ParseStream| {
            while !input.is_empty() {
                let key: Ident = input.parse()?;
                if key == "json_only" {
                    args.json_only = true;
                } else if key == "optional" {
                    let field: Ident = input.parse()?;
                    let when: Ident = input.parse()?;
                    if when != "when" {
                        return Err(syn::Error::new(
                            when.span(),
                            "optional expects `optional FIELD when METHOD`",
                        ));
                    }
                    let method: Ident = input.parse()?;
                    args.optional = Some((field, method));
                } else if key == "check" {
                    let _: syn::Token![=] = input.parse()?;
                    args.check = Some(input.parse()?);
                } else if key == "schema" {
                    let _: syn::Token![=] = input.parse()?;
                    let value: syn::LitBool = input.parse()?;
                    args.generate_schema = value.value;
                } else {
                    return Err(syn::Error::new(
                        key.span(),
                        "unsupported field_codec option; expected json_only, optional FIELD when METHOD, check = path, or schema = bool",
                    ));
                }
                if input.peek(syn::Token![,]) {
                    let _: syn::Token![,] = input.parse()?;
                }
            }
            Ok(())
        })?;
    }
    Ok(args)
}

fn parse_field_wire_override(field: &syn::Field) -> syn::Result<Option<proc_macro2::TokenStream>> {
    let mut wire = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("field_codec") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("wire") {
                let value = meta.value()?;
                let expr: syn::Expr = value.parse()?;
                wire = Some(quote! { ::field::FieldWire::#expr });
                Ok(())
            } else {
                Err(meta.error("unsupported field-level field_codec option; expected wire = ..."))
            }
        })?;
    }
    Ok(wire)
}

fn expand_field_codec(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let args = parse_field_codec_args(&input)?;
    let name = input.ident;
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            input.generics,
            "FieldCodec does not support generic structs",
        ));
    }
    let fields = match input.data {
        Data::Struct(data) => match data.fields {
            Fields::Named(fields) => fields.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    name,
                    "FieldCodec requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                name,
                "FieldCodec can only be derived for structs",
            ));
        }
    };
    if fields.is_empty() {
        return Err(syn::Error::new_spanned(
            name,
            "FieldCodec requires at least one field",
        ));
    }

    let mut named = Vec::new();
    for field in fields {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new(field.span(), "FieldCodec requires named fields"))?;
        let wire_override = parse_field_wire_override(&field)?;
        named.push(NamedField {
            ident,
            ty: field.ty,
            wire_override,
        });
    }

    if let Some((optional_field, _)) = &args.optional {
        if !named.iter().any(|f| f.ident == *optional_field) {
            return Err(syn::Error::new(
                optional_field.span(),
                format!("optional field `{optional_field}` is not a member of {name}"),
            ));
        }
    }

    let json = expand_json(&name, &named, &args);
    let schema = if args.generate_schema {
        expand_schema(&name, &named, &args)
    } else {
        quote! {}
    };
    let rest = if args.json_only {
        quote! {}
    } else {
        expand_codec_and_default(&name, &named)
    };

    Ok(quote! {
        #rest
        #json
        #schema
    })
}

fn expand_codec_and_default(name: &Ident, fields: &[NamedField]) -> proc_macro2::TokenStream {
    let idents: Vec<_> = fields.iter().map(|f| &f.ident).collect();
    let types: Vec<_> = fields.iter().map(|f| &f.ty).collect();
    quote! {
        impl ::field::Encode for #name {
            fn size(&self) -> usize {
                0 #( + ::field::Encode::size(&self.#idents) )*
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                #( ::field::Encode::encode_to(&self.#idents, out); )*
            }
        }

        impl ::field::Decode for #name {
            fn decode(buf: &[u8]) -> ::sys::Ret<(Self, usize)> {
                let mut reader = ::field::Reader::new(buf);
                #( let #idents: #types = reader.read()?; )*
                Ok((Self { #( #idents ),* }, reader.used()))
            }
        }

        impl Default for #name {
            fn default() -> Self {
                Self {
                    #( #idents: Default::default(), )*
                }
            }
        }
    }
}

fn expand_json(
    name: &Ident,
    fields: &[NamedField],
    args: &FieldCodecArgs,
) -> proc_macro2::TokenStream {
    let name_str = name.to_string();
    let optional = args.optional.as_ref();
    let to_json_fields = fields.iter().map(|field| {
        let ident = &field.ident;
        let ident_str = ident.to_string();
        if optional.is_some_and(|(opt, _)| opt == ident) {
            let method = &optional.unwrap().1;
            quote! {
                if self.#method() {
                    s.push('"');
                    s.push_str(#ident_str);
                    s.push_str("\":");
                    s.push_str(&::field::ToJSON::to_json_fmt(&self.#ident, fmt));
                    s.push(',');
                }
            }
        } else {
            quote! {
                s.push('"');
                s.push_str(#ident_str);
                s.push_str("\":");
                s.push_str(&::field::ToJSON::to_json_fmt(&self.#ident, fmt));
                s.push(',');
            }
        }
    });

    let allowed: Vec<_> = fields
        .iter()
        .map(|f| syn::LitStr::new(&f.ident.to_string(), f.ident.span()))
        .collect();
    let match_arms = fields.iter().map(|field| {
        let ident = &field.ident;
        let ident_str = ident.to_string();
        quote! {
            #ident_str => next.#ident.from_json(value)?,
        }
    });
    let require_seen = fields.iter().filter_map(|field| {
        if optional.is_some_and(|(opt, _)| opt == &field.ident) {
            return None;
        }
        let ident_str = field.ident.to_string();
        Some(quote! {
            if !seen.contains(&#ident_str) {
                return ::sys::errf!(
                    "{} JSON missing field {}",
                    #name_str,
                    #ident_str
                );
            }
        })
    });
    let assign = if let Some(check) = &args.check {
        quote! { *self = (#check)(next)?; }
    } else {
        quote! { *self = next; }
    };

    quote! {
        impl ::field::ToJSON for #name {
            fn to_json_fmt(&self, fmt: &::field::JSONFormater) -> String {
                let mut s = String::new();
                s.push('{');
                #( #to_json_fields )*
                if s.len() > 1 {
                    s.pop();
                }
                s.push('}');
                s
            }
        }

        impl ::field::FromJSON for #name {
            fn from_json(&mut self, json: &str) -> ::sys::Ret<()> {
                let mut next = self.clone();
                let mut seen: Vec<&str> = Vec::new();
                ::field::json_object_fields(json, &[#( #allowed ),*], &mut |key, value| {
                    seen.push(key);
                    match key {
                        #( #match_arms )*
                        _ => return ::sys::errf!(
                            "{} JSON field {} is unknown",
                            #name_str,
                            key
                        ),
                    }
                    Ok(())
                })?;
                #( #require_seen )*
                #assign
                Ok(())
            }
        }
    }
}

fn expand_schema(
    name: &Ident,
    fields: &[NamedField],
    args: &FieldCodecArgs,
) -> proc_macro2::TokenStream {
    let name_str = name.to_string();
    let optional = args.optional.as_ref().map(|(field, _)| field);
    let schema_fields = fields.iter().map(|field| {
        let ident_str = field.ident.to_string();
        let wire = field.wire_override.clone().unwrap_or_else(|| {
            let ty = &field.ty;
            quote! { <#ty as ::field::FieldWireShape>::WIRE }
        });
        if optional.is_some_and(|opt| opt == &field.ident) {
            quote! {
                ::field::FieldSchema::optional(#ident_str, #wire)
            }
        } else {
            quote! {
                ::field::FieldSchema::new(#ident_str, #wire)
            }
        }
    });
    quote! {
        impl ::field::FieldWireShape for #name {
            const WIRE: ::field::FieldWire = ::field::FieldWire::Struct(#name_str);
        }

        impl ::field::StructSchemaProvider for #name {
            const STRUCT_SCHEMA: ::field::StructSchema = ::field::StructSchema {
                name: #name_str,
                fields: &[ #( #schema_fields ),* ],
            };
        }

        impl ::field::WireElementName for #name {
            const NAME: &'static str = #name_str;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(src: &str) -> String {
        let input: DeriveInput = syn::parse_str(src).expect("parse struct");
        expand_field_codec(input)
            .expect("expand FieldCodec")
            .to_string()
    }

    #[test]
    fn full_derive_emits_codec_json_default_and_schema() {
        let out =
            expand("#[derive(FieldCodec)] struct AddrHac { address: Address, amount: Amount }");
        assert!(out.contains("impl :: field :: Encode for AddrHac"), "{out}");
        assert!(out.contains("impl :: field :: Decode for AddrHac"), "{out}");
        assert!(out.contains("impl Default for AddrHac"), "{out}");
        assert!(out.contains("impl :: field :: ToJSON for AddrHac"), "{out}");
        assert!(
            out.contains("impl :: field :: FromJSON for AddrHac"),
            "{out}"
        );
        assert!(out.contains("FieldWire :: Struct"), "{out}");
        assert!(
            out.contains("impl :: field :: StructSchemaProvider for AddrHac"),
            "{out}"
        );
        assert!(
            out.contains("impl :: field :: WireElementName for AddrHac"),
            "{out}"
        );
        assert!(out.contains("reader . used ()"), "{out}");
    }

    #[test]
    fn json_only_skips_codec_and_default() {
        let out = expand(
            "#[derive(FieldCodec)] #[field_codec(json_only)] struct Balance { hacash: Amount }",
        );
        assert!(
            !out.contains("impl :: field :: Encode for Balance"),
            "{out}"
        );
        assert!(!out.contains("impl Default for Balance"), "{out}");
        assert!(out.contains("impl :: field :: ToJSON for Balance"), "{out}");
        assert!(out.contains("StructSchemaProvider"), "{out}");
    }

    #[test]
    fn json_only_can_skip_schema() {
        let out = expand(
            "#[derive(FieldCodec)] #[field_codec(json_only, schema = false, check = AssetAmt::checked)] \
             struct AssetAmt { serial: Fold64, amount: Fold64 }",
        );
        assert!(!out.contains("StructSchemaProvider"), "{out}");
        assert!(!out.contains("FieldWireShape"), "{out}");
        assert!(out.contains("AssetAmt :: checked"), "{out}");
    }

    #[test]
    fn optional_omits_when_method_is_false() {
        let out = expand(
            "#[derive(FieldCodec)] #[field_codec(json_only, optional custom_message when has_custom_message)] \
             struct HacdMintData { diamond: DiamondName, custom_message: Hash }",
        );
        assert!(out.contains("has_custom_message"), "{out}");
        assert!(out.contains("FieldSchema :: optional"), "{out}");
        assert!(out.contains("push_str (\"custom_message\")"), "{out}");
        assert!(out.contains("contains (& \"diamond\")"), "{out}");
        assert!(!out.contains("contains (& \"custom_message\")"), "{out}");
    }

    #[test]
    fn field_wire_override_is_emitted() {
        let out = expand(
            "#[derive(FieldCodec)] struct Sign { \
             #[field_codec(wire = Fixed(33))] publickey: Pk, \
             signature: Sig }",
        );
        assert!(out.contains("FieldWire :: Fixed (33)"), "{out}");
    }
}
