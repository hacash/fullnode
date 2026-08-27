use proc_macro::TokenStream;
use quote::ToTokens;
use quote::quote;
use syn::{
    Attribute, Data, DeriveInput, Fields, Ident, ItemStruct, LitInt, Type, Visibility,
    parse::Parser, parse_macro_input, spanned::Spanned,
};

#[derive(Clone)]
enum TransferPayloadSpec {
    Hac(syn::Expr),
    Sat(syn::Expr),
    Asset(syn::Expr, syn::Expr),
    Hacd(syn::Expr, syn::Expr),
}
#[derive(Clone, Default)]
struct TransferSpec {
    to: Option<syn::Expr>,
    from: Option<syn::Expr>,
    payload: Option<TransferPayloadSpec>,
}

#[derive(Clone)]
struct NestedSpec {
    depth: syn::Expr,
    fields: Vec<syn::Ident>,
}

impl syn::parse::Parse for NestedSpec {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let depth: syn::Expr = content.parse()?;
        let _: syn::Token![,] = content.parse()?;
        let mut fields = Vec::new();
        while !content.is_empty() {
            fields.push(content.parse()?);
            if content.peek(syn::Token![,]) {
                let _: syn::Token![,] = content.parse()?;
            }
        }
        if fields.is_empty() {
            return Err(syn::Error::new(
                input.span(),
                "nested requires at least one field",
            ));
        }
        Ok(Self { depth, fields })
    }
}

impl syn::parse::Parse for TransferSpec {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let content;
        syn::parenthesized!(content in input);
        let mut out = TransferSpec::default();
        while !content.is_empty() {
            let key: syn::Ident = content.parse()?;
            let _eq: syn::Token![=] = content.parse()?;
            if key == "to" || key == "from" {
                let expr: syn::Expr = content.parse()?;
                if key == "to" {
                    out.to = Some(expr);
                } else {
                    out.from = Some(expr);
                }
            } else if key == "payload" {
                let kind: syn::Ident = content.parse()?;
                let inner;
                syn::parenthesized!(inner in content);
                let payload = match kind.to_string().as_str() {
                    "Hac" => TransferPayloadSpec::Hac(inner.parse()?),
                    "Sat" => TransferPayloadSpec::Sat(inner.parse()?),
                    "Asset" => {
                        let a = inner.parse()?;
                        let _: syn::Token![,] = inner.parse()?;
                        TransferPayloadSpec::Asset(a, inner.parse()?)
                    }
                    "Hacd" => {
                        let a = inner.parse()?;
                        let _: syn::Token![,] = inner.parse()?;
                        TransferPayloadSpec::Hacd(a, inner.parse()?)
                    }
                    _ => {
                        return Err(syn::Error::new(
                            kind.span(),
                            "payload must be Hac, Sat, Asset, or Hacd",
                        ));
                    }
                };
                out.payload = Some(payload);
            } else {
                return Err(syn::Error::new(
                    key.span(),
                    "transfer expects to, from, or payload",
                ));
            }
            if content.peek(syn::Token![,]) {
                let _: syn::Token![,] = content.parse()?;
            }
        }
        if out.payload.is_none() {
            return Err(syn::Error::new(input.span(), "transfer requires payload"));
        }
        Ok(out)
    }
}

fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 8);
    for (i, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let next = chars.get(i + 1).copied();
            let boundary = prev.is_some_and(|p| p.is_ascii_lowercase())
                || (prev.is_some_and(|p| p.is_ascii_digit())
                    && next.is_some_and(|n| n.is_ascii_lowercase()))
                || (prev.is_some_and(|p| p.is_ascii_uppercase())
                    && next.is_some_and(|n| n.is_ascii_lowercase()));
            if boundary && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::snake_case;
    #[test]
    fn names_cover_acronyms_and_digits() {
        assert_eq!(snake_case("TxMessage"), "tx_message");
        assert_eq!(snake_case("P2SHScriptProve"), "p2sh_script_prove");
        assert_eq!(snake_case("HACDTransfer"), "hacd_transfer");
        assert_eq!(snake_case("V2Action"), "v2_action");
        assert_eq!(snake_case("Message"), "message");
        assert_eq!(snake_case("Blob"), "blob");
        assert_eq!(snake_case("BalanceCoin"), "balance_coin");
        assert_eq!(snake_case("BalanceAsset"), "balance_asset");
        assert_eq!(snake_case("TxBlobSize"), "tx_blob_size");
        assert_eq!(
            snake_case("TransferHacdSingleTo"),
            "transfer_hacd_single_to"
        );
        assert_eq!(snake_case("EnvHeight"), "env_height");
        assert_eq!(snake_case("HacdInscNum"), "hacd_insc_num");
        assert_eq!(snake_case("RequiredSigners"), "required_signers");
        assert_eq!(snake_case("TexCellExecute"), "tex_cell_execute");
        assert_eq!(snake_case("HacdMint"), "hacd_mint");
    }
}

#[cfg(test)]
mod simple_tests {
    use super::*;

    fn expand(src: &str) -> String {
        let input: SimpleAction = syn::parse_str(src).expect("parse simple action");
        expand_simple_action(input)
            .expect("expand simple action")
            .to_string()
    }

    #[test]
    fn fact_bodies_are_bound_to_this() {
        let out = expand(
            "ChannelOpen, 2, 2, TOP, { channel_id: ChannelId }, this, \
             { req_sign: {vec![AddrOrPtr::Addr(this.x)]} }",
        );
        assert!(out.contains("| this : & ChannelOpen |"), "{out}");
        assert!(out.contains("pub const KIND : u16 = 2"), "{out}");
    }

    #[test]
    fn unused_this_is_underscored() {
        let out = expand(
            "EnvHeight, 0x0701, 3, CALL_ONLY, {}, this, \
             { description: \"Syscall\".to_owned() }",
        );
        assert!(out.contains("| _this : & EnvHeight |"), "{out}");
    }

    #[test]
    fn flags_and_verbatim_options_are_forwarded() {
        let out = expand(
            "TxBlob, 0x0402, 2, GUARD, { data: BytesW2 }, this, \
             { blob, name: \"tx_blob\", ctor: none }",
        );
        assert!(out.contains("\"tx_blob\""), "{out}");
        assert!(out.contains("blob"), "{out}");
        assert!(out.contains("pub struct TxBlob"), "{out}");
    }

    #[test]
    fn transfer_spec_is_forwarded_verbatim() {
        let out = expand(
            "HacToTrs, 1, 1, CALL, { to: AddrOrPtr, hacash: Amount }, this, \
             { transfer: (to = to, payload = Hac(hacash)) }",
        );
        assert!(out.contains("TransferPayload :: Hac"), "{out}");
    }

    #[test]
    fn field_visibility_and_attrs_are_preserved() {
        let out = expand(
            "#[derive(PartialEq, Eq)] TexCellAct, 22, 3, TOP, \
             { addr: Address, pub(crate) cells: ListW1<TexCell>, sign: Sign }, this, {}",
        );
        assert!(out.contains("# [derive (PartialEq , Eq)]"), "{out}");
        assert!(
            out.contains("pub (crate) cells : ListW1 < TexCell >"),
            "{out}"
        );
        assert!(out.contains("pub addr : Address"), "{out}");
    }

    #[test]
    fn unknown_options_are_rejected() {
        let input: SimpleAction = syn::parse_str("Foo, 1, 1, TOP, {}, this, { bogus: 1 }").unwrap();
        let err = expand_simple_action(input).unwrap_err();
        assert!(err.to_string().contains("bogus"), "{err}");
    }

    fn expand_entries(src: &str) -> String {
        let input: CodecEntries = syn::parse_str(src).expect("parse codec entries");
        expand_codec_entries(input)
            .expect("expand codec entries")
            .to_string()
    }

    #[test]
    fn codec_entry_names_are_derived_from_the_type() {
        let out =
            expand_entries("HacdMint { wire = (reg, buf) { Ok((reg.as_any(), buf.len())) } }");
        assert!(out.contains("pub fn create_hacd_mint"), "{out}");
        assert!(!out.contains("pub fn decode_hacd_mint_json"), "{out}");
        assert!(out.contains("& dyn :: base :: BinaryCodecs"), "{out}");
        let ast = expand_entries(
            "AstIf { wire = (reg, buf) { Ok((reg.as_any(), buf.len())) }, \
             json = (reg, json) { Ok(reg.as_any()) } }",
        );
        assert!(ast.contains("pub fn create_ast_if"), "{ast}");
        assert!(ast.contains("pub fn decode_ast_if_json"), "{ast}");
        assert!(ast.contains("& dyn :: base :: CodecRegistry"), "{ast}");
    }

    #[test]
    fn codec_entries_json_body_is_optional() {
        let out =
            expand_entries("AstSelect { wire = (reg, buf) { Ok((reg.as_any(), buf.len())) } }");
        assert!(out.contains("pub fn create_ast_select"), "{out}");
        assert!(!out.contains("decode_ast_select_json"), "{out}");
    }
}

/// Generates an action's mechanical codecs (`Default/Encode/Decode/ToJSON/FromJSON`) plus the
/// wire schema (`ACTION_SCHEMA`); field types without a `field::FieldWireShape` impl fail to compile. Review facts come from the definition-site `#[action_codec(...)]` attribute.
#[proc_macro_derive(ActionCodec, attributes(action_codec))]
pub fn derive_action_codec(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_action_codec(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Compact action declaration. The attribute owns the mechanical wire/facts
/// expansion while execution remains in the definition site's
/// `impl_action_execute!` block.
#[proc_macro_attribute]
pub fn action(args: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemStruct);
    match expand_action(args.into(), item) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Call-site sugar over `#[base::action(...)]` for the common action shape:
/// named fields (public by default), `audit = "full"` by default, and fact
/// bodies written against one bound `this` instead of repeated
/// `|this: &Type|` closures.
///
/// ```text
/// base::action_simple! { ChannelOpen, 2, 2, TOP, {
///     channel_id: ChannelId,
///     left_bill:  AddrHac,
///     right_bill: AddrHac
/// }, this, {
///     req_sign:    {vec![AddrOrPtr::Addr(this.left_bill.address), AddrOrPtr::Addr(this.right_bill.address)]},
///     description: format!("Open channel {} for {} and {}", this.channel_id, this.left_bill.address.to_readable(), this.right_bill.address.to_readable())
/// }}
/// ```
///
/// Options: `req_sign` / `description` / `extra9` take a bare body bound to
/// `this` (underscored when the body does not reference it); `name` / `audit` /
/// `validate` / `ctor` / `transfer` take values verbatim; `blob` / `code` are
/// bare flags. Anything else (full-path scopes, `nested`, `wire`) keeps the
/// attribute form.
#[proc_macro]
pub fn action_simple(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as SimpleAction);
    match expand_simple_action(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Codec entry points for manual-wire actions. Generates the `create_<snake>`
/// wire function, and optionally `decode_<snake>_json`, from the action type
/// name — the same snake_case the `action` attribute uses for `NAME` — so a
/// struct rename re-derives the entry names instead of leaving hand-written
/// copies to drift. Custom bodies are written inline next to the type,
/// mirroring `impl_action_execute!`:
///
/// ```text
/// base::action_codec_entries! { AstSelect {
///     wire = (reg, buf) { /* custom wire decode body */ },
///     json = (reg, json) { /* custom JSON decode body */ },
/// }}
/// ```
///
/// The `json` body is optional: omit it when the action uses regular
/// `Default + FromJSON` decoding (e.g. HacdMint). Wire params are
/// `(&dyn base::BinaryCodecs, &[u8])`, JSON params are
/// `(&dyn base::CodecRegistry, &str)`; the caller names them so bodies
/// read naturally and unused ones can be `_`-prefixed.
#[proc_macro]
pub fn action_codec_entries(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as CodecEntries);
    match expand_codec_entries(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

struct CodecEntries {
    ty: Ident,
    wire: (Vec<Ident>, syn::Block),
    json: Option<(Vec<Ident>, syn::Block)>,
}

impl syn::parse::Parse for CodecEntries {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let ty: Ident = input.parse()?;
        let content;
        syn::braced!(content in input);
        let mut wire = None;
        let mut json = None;
        while !content.is_empty() {
            let key: Ident = content.parse()?;
            let _: syn::Token![=] = content.parse()?;
            let params_content;
            syn::parenthesized!(params_content in content);
            let mut params = Vec::new();
            while !params_content.is_empty() {
                params.push(params_content.parse::<Ident>()?);
                if params_content.peek(syn::Token![,]) {
                    let _: syn::Token![,] = params_content.parse()?;
                }
            }
            let body: syn::Block = content.parse()?;
            match key.to_string().as_str() {
                "wire" => wire = Some((params, body)),
                "json" => json = Some((params, body)),
                _ => {
                    return Err(syn::Error::new(
                        key.span(),
                        "action_codec_entries expects wire or json",
                    ));
                }
            }
            if content.peek(syn::Token![,]) {
                let _: syn::Token![,] = content.parse()?;
            }
        }
        let wire = wire.ok_or_else(|| {
            syn::Error::new(ty.span(), "action_codec_entries requires a wire body")
        })?;
        Ok(CodecEntries { ty, wire, json })
    }
}

fn expand_codec_entries(input: CodecEntries) -> syn::Result<proc_macro2::TokenStream> {
    let CodecEntries {
        ty,
        wire: (wparams, wbody),
        json,
    } = input;
    let snake = snake_case(&ty.to_string());
    let create_name = Ident::new(&format!("create_{snake}"), ty.span());
    if wparams.len() != 2 {
        return Err(syn::Error::new(
            ty.span(),
            "wire codec takes two parameters (reg, buf)",
        ));
    }
    let wire_params = wparams.iter().enumerate().map(|(i, p)| {
        let ty = match i {
            0 => quote! { &dyn ::base::BinaryCodecs },
            _ => quote! { &[u8] },
        };
        quote! { #p: #ty }
    });
    let json_fn = if let Some((jparams, jbody)) = json {
        let json_name = Ident::new(&format!("decode_{snake}_json"), ty.span());
        let json_params = jparams.iter().enumerate().map(|(i, p)| {
            let ty = match i {
                0 => quote! { &dyn ::base::CodecRegistry },
                _ => quote! { &str },
            };
            quote! { #p: #ty }
        });
        quote! {
            pub fn #json_name(#(#json_params),*) -> ::sys::Ret<::base::ActionRef> #jbody
        }
    } else {
        quote! {}
    };
    Ok(quote! {
        pub fn #create_name(#(#wire_params),*) -> ::sys::Ret<(::base::ActionRef, usize)> #wbody
        #json_fn
    })
}

struct SimpleAction {
    attrs: Vec<Attribute>,
    name: Ident,
    kind: LitInt,
    tx_min: LitInt,
    scope: Ident,
    fields: Vec<(Visibility, Ident, Type)>,
    this: Ident,
    options: Vec<(Ident, Option<proc_macro2::TokenStream>)>,
}

impl syn::parse::Parse for SimpleAction {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let name: Ident = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let kind: LitInt = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let tx_min: LitInt = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let scope: Ident = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let fields_content;
        syn::braced!(fields_content in input);
        let mut fields = Vec::new();
        while !fields_content.is_empty() {
            let vis: Visibility = fields_content.parse()?;
            let field: Ident = fields_content.parse()?;
            let _: syn::Token![:] = fields_content.parse()?;
            let ty: Type = fields_content.parse()?;
            fields.push((vis, field, ty));
            if fields_content.peek(syn::Token![,]) {
                let _: syn::Token![,] = fields_content.parse()?;
            }
        }
        let _: syn::Token![,] = input.parse()?;
        let this: Ident = input.parse()?;
        let _: syn::Token![,] = input.parse()?;
        let options_content;
        syn::braced!(options_content in input);
        let mut options = Vec::new();
        while !options_content.is_empty() {
            let key: Ident = options_content.parse()?;
            if options_content.peek(syn::Token![:]) {
                let _: syn::Token![:] = options_content.parse()?;
                let value = parse_raw_value(&options_content)?;
                options.push((key, Some(value)));
            } else {
                options.push((key, None));
            }
            if options_content.peek(syn::Token![,]) {
                let _: syn::Token![,] = options_content.parse()?;
            }
        }
        Ok(SimpleAction {
            attrs,
            name,
            kind,
            tx_min,
            scope,
            fields,
            this,
            options,
        })
    }
}

/// Collect the raw tokens of an option value, stopping at a top-level comma.
fn parse_raw_value(input: syn::parse::ParseStream) -> syn::Result<proc_macro2::TokenStream> {
    input.step(|cursor| {
        let mut rest = *cursor;
        let mut out = proc_macro2::TokenStream::new();
        while let Some((tt, next)) = rest.token_tree() {
            if let proc_macro2::TokenTree::Punct(p) = &tt {
                if p.as_char() == ',' {
                    break;
                }
            }
            out.extend(std::iter::once(tt));
            rest = next;
        }
        Ok((out, rest))
    })
}

/// Whether `needle` occurs in `stream`, including inside nested groups.
fn tokens_contain(stream: proc_macro2::TokenStream, needle: &Ident) -> bool {
    for tt in stream {
        match tt {
            proc_macro2::TokenTree::Ident(id) if &id == needle => return true,
            proc_macro2::TokenTree::Group(group) if tokens_contain(group.stream(), needle) => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn expand_simple_action(input: SimpleAction) -> syn::Result<proc_macro2::TokenStream> {
    let SimpleAction {
        attrs,
        name,
        kind,
        tx_min,
        scope,
        fields,
        this,
        options,
    } = input;
    let mut args: Vec<proc_macro2::TokenStream> = vec![
        quote! { kind = #kind },
        quote! { tx_min = #tx_min },
        quote! { scope = #scope },
    ];
    let mut audit_given = false;
    for (key, value) in options {
        let key_name = key.to_string();
        match key_name.as_str() {
            "req_sign" | "description" | "extra9" => {
                let value = value.ok_or_else(|| {
                    syn::Error::new(
                        key.span(),
                        format!("action_simple option {key_name} requires a value"),
                    )
                })?;
                let param = if tokens_contain(value.clone(), &this) {
                    this.clone()
                } else {
                    Ident::new(&format!("_{}", this), this.span())
                };
                args.push(quote! { #key = |#param: &#name| #value });
            }
            "audit" => {
                audit_given = true;
                let value = value.ok_or_else(|| {
                    syn::Error::new(key.span(), "action_simple option audit requires a value")
                })?;
                args.push(quote! { audit = #value });
            }
            "name" | "validate" | "ctor" | "transfer" => {
                let value = value.ok_or_else(|| {
                    syn::Error::new(
                        key.span(),
                        format!("action_simple option {key_name} requires a value"),
                    )
                })?;
                args.push(quote! { #key = #value });
            }
            "blob" | "code" => {
                if value.is_some() {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("action_simple option {key_name} takes no value"),
                    ));
                }
                args.push(quote! { #key });
            }
            _ => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unsupported action_simple option {key_name}"),
                ));
            }
        }
    }
    if !audit_given {
        args.push(quote! { audit = "full" });
    }
    let args = quote! { #(#args),* };
    let fields_tokens: Vec<_> = fields
        .into_iter()
        .map(|(vis, field, ty)| {
            let vis = if matches!(&vis, Visibility::Inherited) {
                quote! { pub }
            } else {
                quote! { #vis }
            };
            quote! { #vis #field: #ty }
        })
        .collect();
    let item: ItemStruct = syn::parse2(quote! {
        #(#attrs)*
        pub struct #name {
            #(#fields_tokens,)*
        }
    })?;
    expand_action(args, item)
}

fn expand_action(
    args: proc_macro2::TokenStream,
    mut item: ItemStruct,
) -> syn::Result<proc_macro2::TokenStream> {
    let mut kind: Option<syn::LitInt> = None;
    let mut tx_min: Option<syn::LitInt> = None;
    let mut scope: Option<syn::Path> = None;
    let mut audit: Option<syn::LitStr> = None;
    let mut blob = false;
    let mut code = false;
    let mut name: Option<syn::LitStr> = None;
    let mut validate: Option<proc_macro2::TokenStream> = None;
    let mut extra9: Option<syn::Expr> = None;
    let mut req_sign: Option<syn::Expr> = None;
    let mut description: Option<syn::Expr> = None;
    let mut transfer: Option<TransferSpec> = None;
    let mut ctor = true;
    let mut wire_manual = false;
    let mut nested: Option<NestedSpec> = None;
    syn::meta::parser(|meta| {
        if meta.path.is_ident("kind") {
            kind = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("tx_min") {
            tx_min = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("scope") {
            scope = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("audit") {
            audit = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("blob") {
            blob = true;
        } else if meta.path.is_ident("code") {
            code = true;
        } else if meta.path.is_ident("name") {
            name = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("validate") {
            let value = meta.value()?;
            if value.peek(syn::LitStr) {
                let lit: syn::LitStr = value.parse()?;
                validate = Some(
                    syn::parse_str::<proc_macro2::TokenStream>(&lit.value()).map_err(|_| {
                        syn::Error::new(lit.span(), "validate must be a valid function path")
                    })?,
                );
            } else {
                let expr: syn::Expr = value.parse()?;
                validate = Some(expr.into_token_stream());
            }
        } else if meta.path.is_ident("extra9") {
            extra9 = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("req_sign") {
            req_sign = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("description") || meta.path.is_ident("desc") {
            description = Some(meta.value()?.parse()?);
        } else if meta.path.is_ident("transfer") {
            if meta.input.peek(syn::Token![=]) {
                let _eq: syn::Token![=] = meta.input.parse()?;
                transfer = Some(meta.input.parse()?);
            } else {
                return Err(meta.error("transfer requires a specification"));
            }
        } else if meta.path.is_ident("ctor") {
            if meta.input.peek(syn::Token![=]) {
                let _eq: syn::Token![=] = meta.input.parse()?;
                let value: syn::Ident = meta.input.parse()?;
                match value.to_string().as_str() {
                    "manual" | "none" => ctor = false,
                    "default" | "auto" => ctor = true,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "ctor must be auto, default, manual, or none",
                        ));
                    }
                }
            } else {
                return Err(meta.error("ctor requires = auto, = default, = manual, or = none"));
            }
        } else if meta.path.is_ident("wire") {
            if meta.input.peek(syn::Token![=]) {
                let _eq: syn::Token![=] = meta.input.parse()?;
                let value: syn::Ident = meta.input.parse()?;
                match value.to_string().as_str() {
                    "manual" => wire_manual = true,
                    "derived" => wire_manual = false,
                    _ => {
                        return Err(syn::Error::new(
                            value.span(),
                            "wire must be derived or manual",
                        ));
                    }
                }
            } else {
                return Err(meta.error("wire requires = derived or = manual"));
            }
        } else if meta.path.is_ident("nested") {
            if meta.input.peek(syn::Token![=]) {
                let _eq: syn::Token![=] = meta.input.parse()?;
                nested = Some(meta.input.parse()?);
            } else {
                return Err(meta.error("nested requires (depth, field, ...)"));
            }
        } else {
            return Err(meta.error("unsupported action option"));
        }
        Ok(())
    })
    .parse2(args)?;
    let kind = kind.ok_or_else(|| syn::Error::new_spanned(&item.ident, "action requires kind"))?;
    let tx_min =
        tx_min.ok_or_else(|| syn::Error::new_spanned(&item.ident, "action requires tx_min"))?;
    let scope =
        scope.ok_or_else(|| syn::Error::new_spanned(&item.ident, "action requires scope"))?;
    let audit =
        audit.ok_or_else(|| syn::Error::new_spanned(&item.ident, "action requires audit"))?;
    let kind_num = kind
        .base10_parse::<u32>()
        .map_err(|_| syn::Error::new_spanned(&kind, "kind must be an integer literal"))?;
    if kind_num > u16::MAX as u32 {
        return Err(syn::Error::new_spanned(&kind, "kind must fit in u16"));
    }
    let scope_expr = if scope.segments.len() == 1 {
        let ident = &scope.segments[0].ident;
        quote! { ::base::ActScope::#ident }
    } else {
        quote! { #scope }
    };
    let ident = item.ident.clone();
    let explicit_name = name.map(|n| n.value());
    let action_name = explicit_name.unwrap_or_else(|| snake_case(&ident.to_string()));
    let name_lit = syn::LitStr::new(&action_name, ident.span());
    let mut codec_args = quote! { audit = #audit, name = #name_lit };
    if blob {
        codec_args.extend(quote! { , blob });
    }
    if code {
        codec_args.extend(quote! { , code });
    }
    if let Some(v) = validate {
        codec_args.extend(quote! { , validate = #v });
    }
    if let syn::Fields::Named(fields) = &mut item.fields {
        let has_kind = fields
            .named
            .first()
            .and_then(|f| f.ident.as_ref())
            .is_some_and(|f| f == "kind");
        if !has_kind {
            fields.named.insert(
                0,
                syn::Field::parse_named.parse2(quote! { pub kind: ::field::Uint2 })?,
            );
        }
    } else {
        return Err(syn::Error::new_spanned(
            &item,
            "action requires named struct fields",
        ));
    }
    let extra_expr = extra9
        .map(|e| match &e {
            syn::Expr::Lit(_) => quote! { #e },
            _ => quote! { (#e)(self) },
        })
        .unwrap_or_else(|| quote! { false });
    let req_expr = req_sign
        .map(|e| match e {
            syn::Expr::Path(p) if p.path.segments.len() == 1 => {
                let field = &p.path.segments[0].ident;
                quote! { vec![self.#field.clone()] }
            }
            other => quote! { (#other)(self) },
        })
        .unwrap_or_else(|| quote! { vec![] });
    let desc_expr = description
        .map(|e| quote! { (#e)(self) })
        .unwrap_or_else(|| quote! { String::new() });
    let transfer_tokens = transfer
        .map(|spec| expand_transfer(&ident, &spec))
        .transpose()?;
    let transfer_impl = if transfer_tokens.is_some() {
        quote! { Some(self) }
    } else {
        quote! { None }
    };
    let attrs = &item.attrs;
    let has_debug = item
        .attrs
        .iter()
        .any(|a| a.path().is_ident("derive") && a.to_token_stream().to_string().contains("Debug"));
    let has_clone = item
        .attrs
        .iter()
        .any(|a| a.path().is_ident("derive") && a.to_token_stream().to_string().contains("Clone"));
    let derive_attr = match (has_debug, has_clone) {
        (true, true) => quote! {},
        (true, false) => quote! { #[derive(Clone)] },
        (false, true) => quote! { #[derive(Debug)] },
        (false, false) => quote! { #[derive(Debug, Clone)] },
    };
    let codec_derive = if wire_manual {
        quote! {}
    } else {
        quote! { #[derive(::base::ActionCodec)] #[action_codec(#codec_args)] }
    };
    let codec_impl = if wire_manual {
        quote! {}
    } else {
        quote! {
            impl ::base::ActionCodec for #ident {
                fn kind(&self) -> u16 { Self::KIND }
                fn schema(&self) -> Option<&'static ::base::ActionSchema> { Some(&<#ident as ::field::ActionSchemaProvider>::ACTION_SCHEMA) }
                fn as_any(&self) -> &dyn std::any::Any { self }
            }
        }
    };
    let vis = &item.vis;
    let fields = &item.fields;
    let ctor_tokens = if ctor {
        expand_ctor(&ident, &item.fields)?
    } else {
        quote! {}
    };
    let nested_tokens = nested
        .map(|spec| {
            let depth = spec.depth;
            let branches = if spec.fields.len() == 1 {
                let field = &spec.fields[0];
                quote! {
                    vec![self.#field.as_list()
                        .iter()
                        .map(|action| action.as_ref() as &dyn ::base::Action)
                        .collect()]
                }
            } else {
                let branch_fields = spec.fields.iter().map(|field| {
                    quote! { self.#field.child_actions() }
                });
                quote! { vec![#(#branch_fields),*] }
            };
            quote! {
                fn nested_actions(&self) -> Option<::base::NestedActions<'_>> {
                    Some(::base::NestedActions {
                        depth_inc: #depth,
                        branches: #branches,
                    })
                }
            }
        })
        .unwrap_or_default();
    Ok(quote! {
        #(#attrs)*
        #derive_attr
        #codec_derive
        #vis struct #ident #fields
        impl #ident {
            pub const KIND: u16 = #kind;
            pub const NAME: &'static str = #name_lit;
            pub const SCOPE: ::base::ActScope = #scope_expr;
        }
        impl ::base::ActionScopeProvider for #ident { const SCOPE: ::base::ActScope = #scope_expr; }
        #codec_impl
        impl ::base::Action for #ident {
            fn scope(&self) -> ::base::ActScope { #scope_expr }
            fn min_tx_type(&self) -> u8 { #tx_min }
            fn extra9(&self) -> bool { #extra_expr }
            fn req_sign(&self) -> Vec<::base::AddrOrPtr> { #req_expr }
            fn description(&self) -> String { #desc_expr }
            fn as_transfer_like(&self) -> Option<&dyn ::base::TransferLike> { #transfer_impl }
            #nested_tokens
            #[cfg(feature = "execute")]
            fn as_execute(&self) -> Option<&dyn ::base::ActionExecute> { Some(self) }
        }
        #transfer_tokens
        #ctor_tokens
    })
}

fn expand_ctor(ident: &syn::Ident, fields: &syn::Fields) -> syn::Result<proc_macro2::TokenStream> {
    let syn::Fields::Named(fields) = fields else {
        return Ok(quote! {});
    };
    let mut params = Vec::new();
    let mut values = Vec::new();
    for f in fields
        .named
        .iter()
        .filter(|f| f.ident.as_ref().is_some_and(|i| i != "kind"))
    {
        let field = f.ident.as_ref().unwrap();
        let mut ty = f.ty.clone();
        let is_addr = field == "to" || field == "from";
        if is_addr && ty.to_token_stream().to_string().ends_with("AddrOrPtr") {
            ty = syn::parse_quote!(::field::Address);
            params.push(quote! { #field: #ty });
            values.push(quote! { ::base::AddrOrPtr::Addr(#field) });
        } else {
            params.push(quote! { #field: #ty });
            values.push(quote! { #field });
        }
    }
    let fields_named: Vec<_> = fields
        .named
        .iter()
        .filter_map(|f| f.ident.as_ref())
        .filter(|i| *i != "kind")
        .collect();
    let assignments: Vec<_> = fields_named
        .iter()
        .zip(values.iter())
        .map(|(field, value)| quote! { #field: #value })
        .collect();
    Ok(
        quote! { impl #ident { pub fn new(#(#params),*) -> Self { Self { kind: ::field::Uint2::from(Self::KIND), #(#assignments),* } } } },
    )
}

fn expand_transfer(
    ident: &syn::Ident,
    spec: &TransferSpec,
) -> syn::Result<proc_macro2::TokenStream> {
    let to = spec.to.as_ref().map(|e| quote! { self.#e.clone() });
    let from = spec.from.as_ref().map(|e| quote! { self.#e.clone() });
    let to_addr = spec.to.as_ref().map(|e| quote! { match self.#e.clone() { ::base::AddrOrPtr::Addr(a) => a, ::base::AddrOrPtr::Ptr(_) => ::field::Address::default() } }).unwrap_or_else(|| quote! { ::field::Address::default() });
    let to_ptr = to
        .clone()
        .map(|e| quote! { Some(#e) })
        .unwrap_or_else(|| quote! { None });
    let from_ptr = from
        .map(|e| quote! { Some(#e) })
        .unwrap_or_else(|| quote! { None });
    let (amount, payload) = match spec.payload.as_ref().expect("validated") {
        TransferPayloadSpec::Hac(e) => (
            quote! { &self.#e },
            quote! { ::base::TransferPayload::Hac { amount: ::field::Encode::encode(&self.#e) } },
        ),
        TransferPayloadSpec::Sat(e) => (
            quote! { ::field::Amount::zero_ref() },
            quote! { ::base::TransferPayload::Sat { satoshi: self.#e.uint() } },
        ),
        TransferPayloadSpec::Asset(serial, amt) => (
            quote! { ::field::Amount::zero_ref() },
            quote! { ::base::TransferPayload::Asset { serial: self.#serial.uint(), amount: self.#amt.uint() } },
        ),
        TransferPayloadSpec::Hacd(count, names) => {
            let names_expr = if matches!(count, syn::Expr::Lit(_)) {
                quote! { self.#names.to_vec() }
            } else {
                quote! { {
                    // The list wire encoding starts with a 1-byte count; `count`
                    // carries it separately, so the payload keeps only the entries.
                    // Guard the assumption where it is cheap instead of relying on it silently.
                    let encoded = ::field::Encode::encode(&self.#names);
                    debug_assert_eq!(
                        encoded.first().copied().unwrap_or(0) as usize,
                        self.#names.length(),
                        "Hacd payload names encoding must start with the count byte"
                    );
                    encoded.get(1..).unwrap_or_default().to_vec()
                } }
            };
            let count_expr = if matches!(count, syn::Expr::Lit(_)) {
                quote! { (#count) as u32 }
            } else {
                quote! { self.#names.length() as u32 }
            };
            (
                quote! { ::field::Amount::zero_ref() },
                quote! { ::base::TransferPayload::Hacd { count: #count_expr, names: #names_expr } },
            )
        }
    };
    Ok(quote! {
        impl ::base::TransferLike for #ident {
            fn transfer_to(&self) -> ::field::Address { #to_addr }
            fn transfer_to_ptr(&self) -> Option<::base::AddrOrPtr> { #to_ptr }
            fn transfer_amount(&self) -> &::field::Amount { #amount }
            fn transfer_from(&self) -> Option<::base::AddrOrPtr> { #from_ptr }
            fn transfer_payload(&self) -> ::base::TransferPayload { #payload }
        }
    })
}

/// Parse the `#[action_codec(...)]` helper attribute (audit class + blob/code flags).
fn parse_action_codec_attr(
    input: &DeriveInput,
) -> syn::Result<(
    String,
    bool,
    bool,
    Option<proc_macro2::TokenStream>,
    Option<String>,
)> {
    let mut audit_class: Option<String> = None;
    let mut blob = false;
    let mut has_code = false;
    let mut validate = None;
    let mut explicit_name = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("action_codec") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("audit") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                let class = lit.value();
                if !matches!(
                    class.as_str(),
                    "full" | "structured" | "branching" | "opaque"
                ) {
                    return Err(meta.error(format!(
                        "invalid audit class {class:?}; expected full|structured|branching|opaque"
                    )));
                }
                audit_class = Some(class);
                Ok(())
            } else if meta.path.is_ident("blob") {
                blob = true;
                Ok(())
            } else if meta.path.is_ident("code") {
                has_code = true;
                Ok(())
            } else if meta.path.is_ident("validate") {
                let value = meta.value()?;
                validate = if value.peek(syn::LitStr) {
                    let lit: syn::LitStr = value.parse()?;
                    Some(syn::parse_str::<proc_macro2::TokenStream>(&lit.value()).map_err(|_| {
                        syn::Error::new(lit.span(), "validate must be a valid function path")
                    })?)
                } else {
                    let path: syn::Path = value.parse()?;
                    Some(path.into_token_stream())
                };
                Ok(())
            } else if meta.path.is_ident("name") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                explicit_name = Some(lit.value());
                Ok(())
            } else {
                Err(meta.error(
                    "unsupported action_codec attribute; expected audit = \"...\", blob, code, or validate = \"path\"",
                ))
            }
        })?;
    }
    let audit_class = audit_class.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "ActionCodec requires #[action_codec(audit = \"full\"|\"structured\"|\"branching\"|\"opaque\")]",
        )
    })?;
    Ok((audit_class, blob, has_code, validate, explicit_name))
}

fn expand_action_codec(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    // Review facts are parsed first (they only need `attrs` + the ident).
    let (audit_class, blob, has_code, validate, explicit_name) = parse_action_codec_attr(&input)?;
    let name = input.ident;
    let auto_name = syn::LitStr::new(
        explicit_name
            .as_deref()
            .unwrap_or(&snake_case(&name.to_string())),
        name.span(),
    );
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

    // wire schema: kind field + FieldWireShape mapping of the remaining fields.
    let mut schema_fields = vec![quote! {
        ::field::FieldSchema::new("kind", ::field::FieldWire::U2)
    }];
    for field in fields.iter().skip(1) {
        let field_name = field
            .ident
            .as_ref()
            .expect("named fields checked above")
            .to_string();
        let name_lit = syn::LitStr::new(&field_name, field.ident.as_ref().expect("named").span());
        let ty = &field.ty;
        schema_fields.push(quote! {
            ::field::FieldSchema::new(
                #name_lit,
                <#ty as ::field::FieldWireShape>::WIRE,
            )
        });
    }

    let schema_fields = schema_fields.as_slice();

    let audit_class = match audit_class.as_str() {
        "full" => quote! { ::field::AuditClass::Full },
        "structured" => quote! { ::field::AuditClass::Structured },
        "branching" => quote! { ::field::AuditClass::Branching },
        "opaque" => quote! { ::field::AuditClass::Opaque },
        _ => unreachable!("validated while parsing"),
    };
    let validate_binary = validate.as_ref().map(|path| quote! { #path(&value)?; });
    let validate_json = validate.as_ref().map(|path| quote! { #path(&value)?; });

    Ok(quote! {
        impl base::ActionName for #name {
            const NAME: &'static str = #auto_name;
        }

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
                let value = Self { kind, #( #value_fields ),* };
                #validate_binary
                Ok((value, reader.used()))
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

        impl Default for #name {
            fn default() -> Self {
                Self {
                    kind: field::Uint2::from(Self::KIND),
                    #( #value_fields: Default::default(), )*
                }
            }
        }

        impl field::FromJSON for #name {
            fn from_json(&mut self, json: &str) -> sys::Ret<()> {
                let mut kind: Option<&str> = None;
                #( let mut #value_fields: Option<#value_types> = None; )*
                field::json_object_fields(json, &["kind", #( #value_names ),*], &mut |key, value| {
                    match key {
                        "kind" => kind = Some(value),
                        #( #value_names => #value_fields = Some(field::json_decode_value(value)?), )*
                        _ => return sys::errf!("action {} JSON field {} is unknown", Self::KIND, key),
                    }
                    Ok(())
                })?;

                let Some(kind_raw) = kind else {
                    return sys::normalf!(
                        "action {} JSON missing required field kind",
                        Self::KIND
                    );
                };
                let kind = field::Uint2::from(field::json_action_kind(
                    kind_raw,
                    <Self as field::ActionSchemaProvider>::ACTION_SCHEMA.name,
                    Self::KIND,
                )?);
                #(
                    let Some(#value_fields) = #value_fields else {
                        return sys::normalf!(
                            "action {} JSON missing required field {}",
                            Self::KIND,
                            #value_names
                        );
                    };
                )*
                let value = Self { kind, #( #value_fields ),* };
                #validate_json
                *self = value;
                Ok(())
            }
        }

        impl field::ActionSchemaProvider for #name {
            const ACTION_SCHEMA: field::ActionSchema = field::ActionSchema {
                kind: Self::KIND,
                name: Self::NAME,
                audit_class: #audit_class,
                blob: #blob,
                has_code: #has_code,
                fields: &[ #(#schema_fields),* ],
            };
        }
    })
}
