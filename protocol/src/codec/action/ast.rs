//! AstSelect / AstIf compositional actions.

use std::sync::Arc;

use base::{Action, ActionRef, AddrOrPtr, BinaryCodecs, CodecRegistry};
use field::{
    Encode, Reader, Uint1, Uint2, json_decode_value, json_object_fields, json_split_array,
};
use sys::Ret;

impl field::ToJSON for ActionListW1 {
    fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
        // `dyn Action` carries JSON rendering in every build (`Action: ToJSON`),
        // so children render as field objects (`{"kind":N,...}`) on SDK/wasm too.
        format!(
            "[{}]",
            self.actions
                .iter()
                .map(|action| action.to_json_fmt(fmt))
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

#[base::action(
    kind = 25,
    tx_min = 3,
    scope = AST,
    audit = "branching",
    wire = manual,
    nested = (1, actions),
    req_sign = |this: &AstSelect| this.collect_req_sign(),
    description = |this: &AstSelect| format!(
        "Execute select {} to {} in {} actions",
        this.exe_min.uint(),
        this.exe_max.uint(),
        this.actions.length()
    ),
    ctor = none,
)]
#[derive(Debug, Clone)]
pub struct AstSelect {
    pub kind: Uint2,
    pub exe_min: Uint1,
    pub exe_max: Uint1,
    pub actions: ActionListW1,
}

#[base::action(
    kind = 26,
    tx_min = 3,
    scope = AST,
    audit = "branching",
    wire = manual,
    nested = (2, cond, br_if, br_else),
    req_sign = |this: &AstIf| this.collect_req_sign(),
    description = |_this: &AstIf| "Asset if-else execute".to_owned(),
    ctor = none,
)]
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

    pub fn push(&mut self, act: ActionRef) -> Ret<()> {
        Uint1::from_usize(self.actions.len() + 1)?;
        self.actions.push(act);
        Ok(())
    }

    pub(crate) fn decode(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let count: Uint1 = r.read()?;
        let mut actions = Vec::with_capacity(count.uint() as usize);
        for _ in 0..count.uint() {
            let act = r.read_with(|rest| reg.decode_action(rest))?;
            actions.push(act);
        }
        Ok((Self { actions }, r.used()))
    }
}

impl AstSelect {
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
    reg.decode_action_json(json)
}

fn decode_ast_select_value(reg: &dyn CodecRegistry, json: &str) -> Ret<AstSelect> {
    let mut kind = None;
    let mut exe_min = None;
    let mut exe_max = None;
    let mut actions = None;
    json_object_fields(
        json,
        &["kind", "exe_min", "exe_max", "actions"],
        &mut |key, value| {
            match key {
                "kind" => kind = Some(value),
                "exe_min" => exe_min = Some(json_decode_value(value)?),
                "exe_max" => exe_max = Some(json_decode_value(value)?),
                "actions" => actions = Some(value),
                _ => return sys::errf!("AstSelect JSON field {} is unknown", key),
            }
            Ok(())
        },
    )?;
    let kind_raw = kind.ok_or_else(|| sys::Error::normal("AstSelect JSON missing kind"))?;
    let kind = Uint2::from(field::json_action_kind(kind_raw, AstSelect::NAME, AstSelect::KIND)?);
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

fn decode_ast_select(reg: &dyn BinaryCodecs, buf: &[u8]) -> Ret<(AstSelect, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != AstSelect::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            AstSelect::KIND,
            kind.uint()
        );
    }
    let exe_min: Uint1 = r.read()?;
    let exe_max: Uint1 = r.read()?;
    let actions = r.read_with(|rest| ActionListW1::decode(reg, rest))?;
    Ok((
        AstSelect {
            kind,
            exe_min,
            exe_max,
            actions,
        },
        r.used(),
    ))
}

// Codec entry points — `create_ast_select` / `decode_ast_select_json` and
// `create_ast_if` / `decode_ast_if_json` are derived from the type names, so a
// struct rename re-derives them instead of leaving hand-written names to drift.
base::action_codec_entries! { AstSelect {
    wire = (reg, buf) {
        let (ast, used) = decode_ast_select(reg, buf)?;
        Ok((Arc::new(ast), used))
    },
    json = (reg, json) {
        Ok(Arc::new(decode_ast_select_value(reg, json)?))
    },
}}

base::action_codec_entries! { AstIf {
    wire = (reg, buf) {
        let mut r = Reader::new(buf);
        let kind: Uint2 = r.read()?;
        if kind.uint() != AstIf::KIND {
            return sys::normalf!(
                "action kind mismatch: expected {} got {}",
                AstIf::KIND,
                kind.uint()
            );
        }
        let cond = r.read_with(|rest| decode_ast_select(reg, rest))?;
        let br_if = r.read_with(|rest| decode_ast_select(reg, rest))?;
        let br_else = r.read_with(|rest| decode_ast_select(reg, rest))?;
        Ok((
            Arc::new(AstIf {
                kind,
                cond,
                br_if,
                br_else,
            }),
            r.used(),
        ))
    },
    json = (reg, json) {
        let mut declared = None;
        let mut cond = None;
        let mut br_if = None;
        let mut br_else = None;
        json_object_fields(
            json,
            &["kind", "cond", "br_if", "br_else"],
            &mut |key, value| {
                match key {
                    "kind" => declared = Some(value),
                    "cond" => cond = Some(value),
                    "br_if" => br_if = Some(value),
                    "br_else" => br_else = Some(value),
                    _ => return sys::errf!("AstIf JSON field {} is unknown", key),
                }
                Ok(())
            },
        )?;
        let declared_raw =
            declared.ok_or_else(|| sys::Error::normal("AstIf JSON missing kind"))?;
        let declared = Uint2::from(field::json_action_kind(declared_raw, AstIf::NAME, AstIf::KIND)?);
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
    },
}}

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
        Uint1::from_usize(self.actions.len())
            .expect("ActionListW1 length overflow")
            .encode_to(out);
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

base::impl_action_codec!(AstSelect);
base::impl_action_codec!(AstIf);

impl base::ActionSchemaProvider for AstSelect {
    const ACTION_SCHEMA: base::ActionSchema = base::ActionSchema {
        kind: Self::KIND,
        name: Self::NAME,
        audit_class: base::AuditClass::Branching,
        blob: false,
        has_code: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("exe_min", base::FieldWire::U8),
            base::FieldSchema::new("exe_max", base::FieldWire::U8),
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
        has_code: false,
        fields: &[
            base::FieldSchema::new("kind", base::FieldWire::U2),
            base::FieldSchema::new("cond", base::FieldWire::Struct("ast_select")),
            base::FieldSchema::new("br_if", base::FieldWire::Struct("ast_select")),
            base::FieldSchema::new("br_else", base::FieldWire::Struct("ast_select")),
        ],
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::action::{TransferHacTo, TransferHacdTo};
    use crate::codec::test_reg::TestRegistry;
    use field::{Address, Amount, Decode, DiamondNameListMax200, Encode, ToJSON};

    #[test]
    fn ast_select_json_emits_nested_child_objects() {
        let child = TransferHacTo::new(Address::default(), Amount::mei(1));
        let ast = AstSelect::create_by(1, 1, vec![Arc::new(child)]).unwrap();
        let json = ast.to_json();
        assert!(
            json.contains("\"kind\":1") && json.contains("\"hacash\""),
            "{json}"
        );
        assert!(!json.contains("\"body\""), "{json}");
    }

    #[test]
    fn ast_select_json_roundtrip_via_nested_children() {
        let reg = TestRegistry::protocol().unwrap();
        let child = TransferHacTo::new(Address::default(), Amount::mei(1));
        let ast = AstSelect::create_by(1, 2, vec![Arc::new(child)]).unwrap();
        let json = ast.to_json();

        let decoded = decode_ast_select_json(&reg, &json).unwrap();
        assert_eq!(decoded.encode(), ast.encode());
        assert_eq!(decoded.to_json(), json);
    }

    #[test]
    fn ast_if_json_roundtrip_with_nested_selects_and_empty_list() {
        let reg = TestRegistry::protocol().unwrap();
        let child = TransferHacTo::new(Address::default(), Amount::mei(1));
        let empty = AstSelect::create_by(0, 0, vec![]).unwrap();
        let cond = AstSelect::create_by(1, 1, vec![Arc::new(child)]).unwrap();
        let ast = AstIf::create_by(cond, empty, AstSelect::create_by(1, 1, vec![]).unwrap());
        let json = ast.to_json();

        let decoded = decode_ast_if_json(&reg, &json).unwrap();
        assert_eq!(decoded.encode(), ast.encode());
        assert_eq!(decoded.to_json(), json);
    }

    #[test]
    fn ast_child_rejects_body_hex_form() {
        let reg = TestRegistry::protocol().unwrap();
        // The `{"body":"0x..."}` shim was removed: children must carry `kind`.
        let json = "{\"kind\":25,\"exe_min\":1,\"exe_max\":1,\"actions\":[{\"body\":\"0x00\"}]}";
        assert!(decode_ast_select_json(&reg, json).is_err());
    }

    #[test]
    fn ast_select_json_roundtrip_nested_diamond_transfer_csv() {
        let reg = TestRegistry::protocol().unwrap();
        let diamonds = DiamondNameListMax200::from_readable("WTYUIA,HYXYHY").unwrap();
        let child = TransferHacdTo::new(Address::default(), diamonds);
        let ast = AstSelect::create_by(1, 1, vec![Arc::new(child)]).unwrap();
        let json = ast.to_json();
        assert!(json.contains("\"diamonds\":\"WTYUIA,HYXYHY\""), "{json}");
        let decoded = decode_ast_select_json(&reg, &json).unwrap();
        assert_eq!(decoded.encode(), ast.encode());
        assert_eq!(decoded.to_json(), json);
    }

    fn child() -> TransferHacTo {
        TransferHacTo::new(Address::default(), Amount::mei(1))
    }

    #[test]
    fn action_list_w1_count_bounds() {
        let empty = ActionListW1::from_vec(vec![]).unwrap();
        assert_eq!(empty.size(), empty.encode().len());
        let (decoded, used) =
            ActionListW1::decode(&TestRegistry::protocol().unwrap(), &empty.encode()).unwrap();
        assert_eq!(used, empty.encode().len());
        assert_eq!(decoded.length(), 0);

        let one = ActionListW1::from_vec(vec![Arc::new(child())]).unwrap();
        assert_eq!(one.size(), one.encode().len());
        let (decoded, used) =
            ActionListW1::decode(&TestRegistry::protocol().unwrap(), &one.encode()).unwrap();
        assert_eq!(used, one.encode().len());
        assert_eq!(decoded.length(), 1);

        let max =
            ActionListW1::from_vec((0..u8::MAX).map(|_| Arc::new(child()) as _).collect()).unwrap();
        assert_eq!(max.length(), u8::MAX as usize);
        assert_eq!(max.size(), max.encode().len());
        assert!(
            ActionListW1::from_vec(
                (0..u8::MAX as usize + 1)
                    .map(|_| Arc::new(child()) as _)
                    .collect()
            )
            .is_err()
        );
        let mut list =
            ActionListW1::from_vec((0..u8::MAX).map(|_| Arc::new(child()) as _).collect()).unwrap();
        assert!(list.push(Arc::new(child())).is_err());
    }

    #[test]
    fn ast_wire_roundtrip_and_wrong_kind() {
        let reg = TestRegistry::protocol().unwrap();
        let ast = AstSelect::create_by(1, 1, vec![Arc::new(child())]).unwrap();
        assert_eq!(ast.size(), ast.encode().len());
        let wire = ast.encode();
        let (decoded, used) = decode_ast_select(&reg, &wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(decoded.encode(), wire);
        let (via_create, used) = create_ast_select(&reg, &wire).unwrap();
        assert_eq!(used, wire.len());
        assert_eq!(via_create.encode(), wire);

        let mut wrong = wire.clone();
        wrong[1] = 1;
        assert!(decode_ast_select(&reg, &wrong).is_err());
        assert!(create_ast_select(&reg, &wrong).is_err());
        assert!(TransferHacTo::decode(&wire).is_err());
    }
}
