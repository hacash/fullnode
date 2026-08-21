//! AstSelect / AstIf compositional actions.

use std::any::Any;
use std::sync::Arc;

use base::{
    ActScope, Action, ActionCodec, ActionRef, AddrOrPtr, BinaryCodecs, CodecRegistry, TopRule,
};
#[cfg(feature = "execute")]
use base::{ActionExecute, ActionJsonView};
use field::{
    Decode, Encode, Reader, Uint1, Uint2, json_decode_value, json_expect_unquoted,
    json_object_entries, json_object_fields, json_split_array,
};
use sys::Ret;

impl field::ToJSON for ActionListW1 {
    fn to_json_fmt(&self, _fmt: &field::JSONFormater) -> String {
        // `Action` has no JSON view (SDK wasm core is JSON-free): serialize each child
        // from its wire form as `{"body":"<hex>"}`, decoded by `decode_ast_child` via `decode_action_exact`.
        format!(
            "[{}]",
            self.actions
                .iter()
                .map(|action| format!("{{\"body\":\"0x{}\"}}", hex::encode(action.encode())))
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

impl field::FieldWireShape for ActionListW1 {
    const WIRE: field::FieldWire = field::FieldWire::ActionListW1;
}

field::impl_action_json!(AstSelect {
    exe_min,
    exe_max,
    actions
});
field::impl_action_json!(AstIf {
    cond,
    br_if,
    br_else
});

#[derive(Debug, Clone)]
pub struct ActionListW1 {
    actions: Vec<ActionRef>,
}

#[derive(Debug, Clone)]
pub struct AstSelect {
    pub kind: Uint2,
    pub exe_min: Uint1,
    pub exe_max: Uint1,
    pub actions: ActionListW1,
}

#[derive(Debug, Clone)]
pub struct AstIf {
    pub kind: Uint2,
    pub cond: AstSelect,
    pub br_if: AstSelect,
    pub br_else: AstSelect,
}

impl ActionListW1 {
    pub fn from_vec(actions: Vec<ActionRef>) -> Ret<Self> {
        Uint1::from_usize(actions.len())?;
        Ok(Self { actions })
    }

    pub fn as_list(&self) -> &[ActionRef] {
        &self.actions
    }

    pub fn length(&self) -> usize {
        self.actions.len()
    }

    fn decode(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(Self, usize)> {
        let (count, mut used) = Uint1::decode(buf)?;
        let mut actions = Vec::with_capacity(count.uint() as usize);
        for _ in 0..count.uint() {
            let rest = &buf[used..];
            let (act, n) = reg.decode_action(rest)?;
            actions.push(act);
            used += n;
        }
        Ok((Self { actions }, used))
    }
}

impl AstSelect {
    pub const KIND: u16 = 25;
    pub const NAME: &'static str = "ast_select";

    pub fn create_by(min: u8, max: u8, actions: Vec<ActionRef>) -> Ret<Self> {
        Ok(Self {
            kind: Uint2::from(Self::KIND),
            exe_min: Uint1::from(min),
            exe_max: Uint1::from(max),
            actions: ActionListW1::from_vec(actions)?,
        })
    }

    fn collect_req_sign(&self) -> Vec<AddrOrPtr> {
        let mut req = vec![];
        for act in self.actions.as_list() {
            collect_ast_req_sign(&mut req, act.as_ref());
        }
        req
    }

    pub(crate) fn child_actions(&self) -> Vec<&dyn Action> {
        self.actions
            .as_list()
            .iter()
            .map(|a| a.as_ref() as &dyn Action)
            .collect()
    }
}

impl AstIf {
    pub const KIND: u16 = 26;
    pub const NAME: &'static str = "ast_if";

    pub fn create_by(cond: AstSelect, br_if: AstSelect, br_else: AstSelect) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            cond,
            br_if,
            br_else,
        }
    }

    fn collect_req_sign(&self) -> Vec<AddrOrPtr> {
        let mut req = self.cond.collect_req_sign();
        req.extend(self.br_if.collect_req_sign());
        req.extend(self.br_else.collect_req_sign());
        req
    }
}

fn decode_ast_child(reg: &dyn CodecRegistry, json: &str) -> Ret<ActionRef> {
    let entries = json_object_entries(json)?;
    if let Some((_, body)) = entries.iter().find(|(key, _)| *key == "body") {
        return reg.decode_action_exact(&field::json_decode_binary(body)?);
    }
    let kind = entries
        .iter()
        .find(|(key, _)| *key == "kind")
        .map(|(_, value)| *value)
        .ok_or_else(|| sys::Error::fault("AST child action missing kind"))?;
    let kind: u16 = json_expect_unquoted(kind)?
        .parse()
        .map_err(|_| sys::Error::normal("AST child action kind invalid"))?;
    reg.decode_action_json(kind, json)?.ok_or_else(|| {
        sys::Error::normal(format!("AST child action kind {} has no JSON codec", kind))
    })
}

fn decode_ast_select_value(reg: &dyn CodecRegistry, json: &str) -> Ret<AstSelect> {
    let mut kind = Uint2::from(AstSelect::KIND);
    let mut exe_min = None;
    let mut exe_max = None;
    let mut actions = None;
    json_object_fields(
        json,
        &["kind", "exe_min", "exe_max", "actions"],
        |key, value| {
            match key {
                "kind" => kind = json_decode_value(value)?,
                "exe_min" => exe_min = Some(json_decode_value(value)?),
                "exe_max" => exe_max = Some(json_decode_value(value)?),
                "actions" => actions = Some(value),
                _ => unreachable!("allowed field checked by json_object_fields"),
            }
            Ok(())
        },
    )?;
    if kind.uint() != AstSelect::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            AstSelect::KIND,
            kind.uint()
        );
    }
    let exe_min: Uint1 =
        exe_min.ok_or_else(|| sys::Error::normal("AstSelect JSON missing exe_min"))?;
    let exe_max: Uint1 =
        exe_max.ok_or_else(|| sys::Error::normal("AstSelect JSON missing exe_max"))?;
    let actions_json =
        actions.ok_or_else(|| sys::Error::normal("AstSelect JSON missing actions"))?;
    let mut children = Vec::new();
    for child in json_split_array(actions_json)? {
        children.push(decode_ast_child(reg, child)?);
    }
    Ok(AstSelect {
        kind,
        exe_min,
        exe_max,
        actions: ActionListW1::from_vec(children)?,
    })
}

pub fn decode_ast_select_json(reg: &dyn CodecRegistry, kind: u16, json: &str) -> Ret<ActionRef> {
    if kind != AstSelect::KIND {
        return sys::normalf!("AstSelect JSON codec got kind {}", kind);
    }
    Ok(Arc::new(decode_ast_select_value(reg, json)?))
}

pub fn decode_ast_if_json(reg: &dyn CodecRegistry, kind: u16, json: &str) -> Ret<ActionRef> {
    if kind != AstIf::KIND {
        return sys::normalf!("AstIf JSON codec got kind {}", kind);
    }
    let mut declared = Uint2::from(AstIf::KIND);
    let mut cond = None;
    let mut br_if = None;
    let mut br_else = None;
    json_object_fields(json, &["kind", "cond", "br_if", "br_else"], |key, value| {
        match key {
            "kind" => declared = json_decode_value(value)?,
            "cond" => cond = Some(value),
            "br_if" => br_if = Some(value),
            "br_else" => br_else = Some(value),
            _ => unreachable!("allowed field checked by json_object_fields"),
        }
        Ok(())
    })?;
    if declared.uint() != AstIf::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            AstIf::KIND,
            declared.uint()
        );
    }
    Ok(Arc::new(AstIf {
        kind: declared,
        cond: decode_ast_select_value(
            reg,
            cond.ok_or_else(|| sys::Error::normal("AstIf JSON missing cond"))?,
        )?,
        br_if: decode_ast_select_value(
            reg,
            br_if.ok_or_else(|| sys::Error::normal("AstIf JSON missing br_if"))?,
        )?,
        br_else: decode_ast_select_value(
            reg,
            br_else.ok_or_else(|| sys::Error::normal("AstIf JSON missing br_else"))?,
        )?,
    }))
}

fn collect_ast_req_sign(req: &mut Vec<AddrOrPtr>, act: &dyn Action) {
    if let Some(nested) = act.nested_actions() {
        for child in nested.flatten() {
            collect_ast_req_sign(req, child);
        }
        return;
    }
    req.extend(act.req_sign());
}

impl Encode for ActionListW1 {
    fn size(&self) -> usize {
        Uint1::SIZE + self.actions.iter().map(|a| a.size()).sum::<usize>()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        Uint1::from(self.actions.len() as u8).encode_to(out);
        for action in &self.actions {
            action.encode_to(out);
        }
    }
}

impl Encode for AstSelect {
    fn size(&self) -> usize {
        self.kind.size() + self.exe_min.size() + self.exe_max.size() + self.actions.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.exe_min.encode_to(out);
        self.exe_max.encode_to(out);
        self.actions.encode_to(out);
    }
}

impl Encode for AstIf {
    fn size(&self) -> usize {
        self.kind.size() + self.cond.size() + self.br_if.size() + self.br_else.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.cond.encode_to(out);
        self.br_if.encode_to(out);
        self.br_else.encode_to(out);
    }
}

impl ActionCodec for AstSelect {
    fn kind(&self) -> u16 {
        Self::KIND
    }

    fn schema(&self) -> Option<&'static base::ActionSchema> {
        Some(&<Self as base::ActionSchemaProvider>::ACTION_SCHEMA)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl base::ActionScopeProvider for AstSelect {
    const SCOPE: ActScope = ActScope::AST;
}

impl Action for AstSelect {
    fn scope(&self) -> ActScope {
        ActScope {
            top: Some(TopRule::None),
            allow_ast: true,
            allow_call: false,
        }
    }

    fn min_tx_type(&self) -> u8 {
        3
    }

    fn description(&self) -> String {
        format!(
            "Execute select {} to {} in {} actions",
            self.exe_min.uint(),
            self.exe_max.uint(),
            self.actions.length()
        )
    }

    fn req_sign(&self) -> Vec<AddrOrPtr> {
        self.collect_req_sign()
    }

    fn nested_actions(&self) -> Option<base::NestedActions<'_>> {
        Some(base::NestedActions {
            depth_inc: 1,
            branches: vec![self.child_actions()],
        })
    }

    #[cfg(feature = "execute")]
    fn as_execute(&self) -> Option<&dyn ActionExecute> {
        Some(self)
    }

    #[cfg(feature = "execute")]
    fn as_json_view(&self) -> Option<&dyn ActionJsonView> {
        Some(self)
    }
}

impl ActionCodec for AstIf {
    fn kind(&self) -> u16 {
        Self::KIND
    }

    fn schema(&self) -> Option<&'static base::ActionSchema> {
        Some(&<Self as base::ActionSchemaProvider>::ACTION_SCHEMA)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl base::ActionScopeProvider for AstIf {
    const SCOPE: ActScope = ActScope::AST;
}

impl Action for AstIf {
    fn scope(&self) -> ActScope {
        ActScope {
            top: Some(TopRule::None),
            allow_ast: true,
            allow_call: false,
        }
    }

    fn min_tx_type(&self) -> u8 {
        3
    }

    fn description(&self) -> String {
        "Asset if-else execute".to_owned()
    }

    fn req_sign(&self) -> Vec<AddrOrPtr> {
        self.collect_req_sign()
    }

    fn nested_actions(&self) -> Option<base::NestedActions<'_>> {
        Some(base::NestedActions {
            depth_inc: 2,
            branches: vec![
                self.cond.child_actions(),
                self.br_if.child_actions(),
                self.br_else.child_actions(),
            ],
        })
    }

    #[cfg(feature = "execute")]
    fn as_execute(&self) -> Option<&dyn ActionExecute> {
        Some(self)
    }

    #[cfg(feature = "execute")]
    fn as_json_view(&self) -> Option<&dyn ActionJsonView> {
        Some(self)
    }
}

pub fn create_ast_select(
    reg: &dyn BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != AstSelect::KIND {
        return sys::normalf!("AstSelect codec got kind {}", kind.uint());
    }
    let exe_min: Uint1 = r.read()?;
    let exe_max: Uint1 = r.read()?;
    let (actions, used) = ActionListW1::decode(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    Ok((
        Arc::new(AstSelect {
            kind,
            exe_min,
            exe_max,
            actions,
        }),
        r.used(),
    ))
}

fn decode_ast_select_inline(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(AstSelect, usize)> {
    let (act, used) = create_ast_select(reg, AstSelect::KIND, buf)?;
    let Some(ast) = act.as_any().downcast_ref::<AstSelect>() else {
        return sys::normalf!("AstSelect decode type mismatch");
    };
    Ok((ast.clone(), used))
}

pub fn create_ast_if(reg: &dyn BinaryCodecs, _kind: u16, buf: &[u8]) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != AstIf::KIND {
        return sys::normalf!("AstIf codec got kind {}", kind.uint());
    }
    let (cond, used) = decode_ast_select_inline(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    let (br_if, used) = decode_ast_select_inline(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    let (br_else, used) = decode_ast_select_inline(reg, &buf[r.used()..])?;
    r.read_bytes(used)?;
    Ok((
        Arc::new(AstIf {
            kind,
            cond,
            br_if,
            br_else,
        }),
        r.used(),
    ))
}

// ================================ wire schema ================================

impl base::ActionSchemaProvider for AstSelect {
    const ACTION_SCHEMA: base::ActionSchema = base::ActionSchema {
        kind: Self::KIND,
        name: Self::NAME,
        audit_class: base::AuditClass::Branching,
        blob: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("exe_min", base::FieldWire::U1),
            base::FieldSchema::new("exe_max", base::FieldWire::U1),
            // `ActionListW1`: the actual wire is a 1-byte count (Uint1), unlike
            // `ActionListW2`'s 2-byte count.
            base::FieldSchema::new("actions", base::FieldWire::ActionListW1),
        ],
    };
}

impl base::ActionSchemaProvider for AstIf {
    const ACTION_SCHEMA: base::ActionSchema = base::ActionSchema {
        kind: Self::KIND,
        name: Self::NAME,
        audit_class: base::AuditClass::Branching,
        blob: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("cond", base::FieldWire::Struct("ast_select")),
            base::FieldSchema::new("br_if", base::FieldWire::Struct("ast_select")),
            base::FieldSchema::new("br_else", base::FieldWire::Struct("ast_select")),
        ],
    };
}
