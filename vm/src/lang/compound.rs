//! Compiler-only stack fragments for compound assignments.
//!
//! `l[i] op= v` and the `idx >= 64` local `x op= v` fallback both lower to a
//! fixed `IRNodeBytecodes` fragment. The lowering and the decompiler's pattern
//! matcher must agree byte-for-byte; keep the fragment and its parser here so
//! they cannot drift apart.

use crate::rt::Bytecode;

/// `DUPN 2; ITEMGET; ROLL 3; op; ITEMSET`
pub(crate) const ITEM_COMPOUND_RAW_LEN: usize = 7;

/// `GET idx; SWAP; op; PUT idx`
pub(crate) const LOCAL_COMPOUND_RAW_LEN: usize = 6;

/// Slots below this use the `XOP` encoding; the raw local fallback is only
/// generated for slots at or above it.
pub(crate) const LOCAL_COMPOUND_XOP_LIMIT: u8 = 64;

/// Build the item-compound fragment. The caller emits `value`, `receiver`,
/// `index` first (RHS-first); this fragment duplicates receiver+index, reads
/// the element, rolls the RHS to the top, applies `op`, and writes back.
pub(crate) fn item_compound_raw(op: Bytecode) -> Option<[u8; ITEM_COMPOUND_RAW_LEN]> {
    use Bytecode::*;
    let op = match op {
        ADD | SUB | MUL | DIV => op as u8,
        _ => return None,
    };
    Some([
        DUPN as u8,
        2,
        ITEMGET as u8,
        ROLL as u8,
        3,
        op,
        ITEMSET as u8,
    ])
}

/// Recognize the exact compiler-generated item fragment and return its
/// arithmetic opcode. Anything else is not a canonical `l[i] op= v` lowering.
pub(crate) fn item_compound_op(codes: &[u8]) -> Option<Bytecode> {
    if codes.len() != ITEM_COMPOUND_RAW_LEN {
        return None;
    }
    let op = Bytecode::try_from_u8(codes[5]).ok()?;
    let want = item_compound_raw(op)?;
    (codes == &want[..]).then_some(op)
}

/// Build the local fallback for slots without an `XOP` encoding. `idx >= 64`
/// is required; the caller emits `value` first, then this fragment.
pub(crate) fn local_compound_raw(
    op: Bytecode,
    idx: u8,
) -> Option<[u8; LOCAL_COMPOUND_RAW_LEN]> {
    use Bytecode::*;
    if idx < LOCAL_COMPOUND_XOP_LIMIT {
        return None;
    }
    let op = match op {
        ADD | SUB | MUL | DIV => op as u8,
        _ => return None,
    };
    Some([GET as u8, idx, SWAP as u8, op, PUT as u8, idx])
}

/// Recognize the exact compiler-generated high-slot local fragment and return
/// `(slot, arithmetic opcode)`. Low slots are rejected because those use XOP.
pub(crate) fn local_compound_parts(codes: &[u8]) -> Option<(u8, Bytecode)> {
    if codes.len() != LOCAL_COMPOUND_RAW_LEN {
        return None;
    }
    let idx = codes[1];
    let op = Bytecode::try_from_u8(codes[3]).ok()?;
    let want = local_compound_raw(op, idx)?;
    (codes == &want[..]).then_some((idx, op))
}

/// Source spelling of the arithmetic assignment operator.
pub(crate) fn assign_symbol(op: Bytecode) -> Option<&'static str> {
    use Bytecode::*;
    Some(match op {
        ADD => "+=",
        SUB => "-=",
        MUL => "*=",
        DIV => "/=",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_fragment_roundtrips_all_arithmetic_ops() {
        for op in [Bytecode::ADD, Bytecode::SUB, Bytecode::MUL, Bytecode::DIV] {
            let raw = item_compound_raw(op).unwrap();
            assert_eq!(item_compound_op(&raw), Some(op));
            assert_eq!(assign_symbol(op).is_some(), true);
        }
        assert_eq!(item_compound_op(&[0u8; ITEM_COMPOUND_RAW_LEN]), None);
        assert_eq!(assign_symbol(Bytecode::ITEMSET), None);
    }

    #[test]
    fn local_fragment_is_high_slot_only() {
        assert_eq!(local_compound_raw(Bytecode::ADD, 63), None);
        let low = [
            Bytecode::GET as u8,
            63,
            Bytecode::SWAP as u8,
            Bytecode::ADD as u8,
            Bytecode::PUT as u8,
            63,
        ];
        assert_eq!(local_compound_parts(&low), None);
        assert_eq!(
            local_compound_parts(&local_compound_raw(Bytecode::ADD, 64).unwrap()),
            Some((64, Bytecode::ADD))
        );
        let raw = local_compound_raw(Bytecode::SUB, 200).unwrap();
        assert_eq!(local_compound_parts(&raw), Some((200, Bytecode::SUB)));
    }
}
