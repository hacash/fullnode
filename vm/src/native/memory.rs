//! Contract memory policy operations. The primitive map ops stay opcodes;
//! this module is the NativeCtl layer (check-and-set and later siblings).

use field::Address;

use crate::rt::{EffectMode, ExecCtx, GasExtra, ItrErr, ItrErrCode, SpaceCap, VmrtRes};
use crate::space::{validate_volatile_kv_put, CtcKVMap, VolatileKvLimits};
use crate::value::Value;

use super::NativeCtl;

/// `memory_init(key) -> Bool`. Edit only.
///
/// Absent key: store `true` and return `true`. Present key: leave the value
/// unchanged and return `false`. A later `memory_put(key, nil)` deletes the
/// slot, so a following init can succeed again.
pub fn call_memory_init(
    exec: ExecCtx,
    cap: &SpaceCap,
    gst: &GasExtra,
    context_addr: &Address,
    memory: &mut CtcKVMap,
    key: Value,
) -> VmrtRes<(Value, i64)> {
    if exec.effect != EffectMode::Edit {
        return itr_err_code!(ItrErrCode::InstDisabled);
    }
    let stored = Value::Bool(true);
    let limits = VolatileKvLimits::from_space_cap(cap);
    validate_volatile_kv_put(&key, &stored, &limits, false, ItrErrCode::MemoryError)?;
    let klen = key
        .extract_key_bytes_with_error_code(ItrErrCode::MemoryError)?
        .len();
    let exists = match memory.entry(context_addr)? {
        Some(mem) => mem.contains_key(&key)?,
        None => false,
    };
    let mut gas = NativeCtl::memory_init
        .gas_of()
        .saturating_add(gst.stack_write(klen));
    if exists {
        return Ok((Value::Bool(false), gas));
    }
    gas = gas
        .saturating_add(gst.stack_write(stored.val_size()))
        .saturating_add(gst.memory_key_cost);
    memory.entry_mut(context_addr)?.put(key, stored)?;
    Ok((Value::Bool(true), gas))
}

#[cfg(test)]
mod tests {
    use field::Address;

    use super::*;
    use crate::machine::test_ctx::STUB_VM_PARAMS;
    use crate::rt::{EntryKind, ItrErrCode};
    use crate::value::Value;

    fn addr(tag: u8) -> Address {
        let mut raw = [0u8; 21];
        raw[0] = Address::VERSION_CONTRACT;
        raw[20] = tag;
        Address::from(raw)
    }

    fn gst() -> GasExtra {
        GasExtra::new(0, &STUB_VM_PARAMS)
    }

    fn map(limit: usize) -> CtcKVMap {
        let cap = SpaceCap::new(0);
        CtcKVMap::with_key_max(limit, cap.kv_key_size)
    }

    fn init(
        effect: EffectMode,
        memory: &mut CtcKVMap,
        who: Address,
        key: Value,
    ) -> Result<(Value, i64), ItrErr> {
        let cap = SpaceCap::new(0);
        call_memory_init(
            ExecCtx::new(EntryKind::Main, effect, 1),
            &cap,
            &gst(),
            &who,
            memory,
            key,
        )
    }

    #[test]
    fn catalog_pins_memory_init() {
        assert_eq!(NativeCtl::memory_init as u8, 81);
        assert_eq!(NativeCtl::argv_len(NativeCtl::memory_init as u8), Some(1));
        assert_eq!(NativeCtl::memory_init.rty_of(), crate::value::ValueTy::Bool);
        // Catalog base price: the hit path pays exactly this; the insert path is
        // surcharged at runtime with stack_write(bool) + memory_key_cost (see
        // call_memory_init), so the row itself stays in the check-op band.
        assert_eq!(NativeCtl::memory_init.gas_of(), 8);
        assert_eq!(NativeCtl::from_name("memory_init").map(|v| v.0), Some(81));
        assert!(!NativeCtl::has_idx(82));
    }

    #[test]
    fn inserts_true_once_and_does_not_clobber() {
        let who = addr(1);
        let mut memory = map(24);
        let (v, insert_gas) = init(EffectMode::Edit, &mut memory, who, Value::U8(0)).unwrap();
        assert_eq!(v, Value::Bool(true));
        assert_eq!(memory.get(&who, &Value::U8(0)).unwrap(), Value::Bool(true));

        let (v, hit_gas) = init(EffectMode::Edit, &mut memory, who, Value::U8(0)).unwrap();
        assert_eq!(v, Value::Bool(false));
        assert_eq!(memory.get(&who, &Value::U8(0)).unwrap(), Value::Bool(true));

        let g = gst();
        assert_eq!(
            insert_gas - hit_gas,
            g.memory_key_cost + g.stack_write(Value::Bool(true).val_size())
        );

        memory
            .entry_mut(&who)
            .unwrap()
            .put(Value::U8(1), Value::Bool(false))
            .unwrap();
        let (v, _) = init(EffectMode::Edit, &mut memory, who, Value::U8(1)).unwrap();
        assert_eq!(v, Value::Bool(false));
        assert_eq!(memory.get(&who, &Value::U8(1)).unwrap(), Value::Bool(false));

        memory.remove(&who, &Value::U8(0)).unwrap();
        let (v, _) = init(EffectMode::Edit, &mut memory, who, Value::U8(0)).unwrap();
        assert_eq!(v, Value::Bool(true));
    }

    #[test]
    fn partitions_by_context_address() {
        let mut memory = map(24);
        assert_eq!(
            init(EffectMode::Edit, &mut memory, addr(1), Value::U8(0))
                .unwrap()
                .0,
            Value::Bool(true)
        );
        assert_eq!(
            init(EffectMode::Edit, &mut memory, addr(2), Value::U8(0))
                .unwrap()
                .0,
            Value::Bool(true)
        );
    }

    #[test]
    fn rejects_non_edit_before_touching_the_map() {
        let who = addr(1);
        let mut memory = map(24);
        for effect in [EffectMode::Pure, EffectMode::View] {
            let err = init(effect, &mut memory, who, Value::Nil).unwrap_err();
            assert_eq!(err.0, ItrErrCode::InstDisabled, "{effect:?} {err:?}");
        }
        assert_eq!(memory.addr_len(), 0);
        let err = init(EffectMode::Edit, &mut memory, who, Value::Nil).unwrap_err();
        assert_eq!(err.0, ItrErrCode::MemoryError);
        let err = init(EffectMode::Edit, &mut memory, who, Value::Bool(true)).unwrap_err();
        assert_eq!(err.0, ItrErrCode::MemoryError);
        let err = init(
            EffectMode::Edit,
            &mut memory,
            who,
            Value::Bytes(vec![1u8; 129]),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::MemoryError);
        assert_eq!(memory.addr_len(), 0);
    }

    #[test]
    fn full_map_is_out_of_memory_not_false() {
        let who = addr(1);
        let mut memory = map(1);
        assert_eq!(
            init(EffectMode::Edit, &mut memory, who, Value::U8(0))
                .unwrap()
                .0,
            Value::Bool(true)
        );
        assert_eq!(
            init(EffectMode::Edit, &mut memory, who, Value::U8(0))
                .unwrap()
                .0,
            Value::Bool(false)
        );
        let err = init(EffectMode::Edit, &mut memory, who, Value::U8(1)).unwrap_err();
        assert_eq!(err.0, ItrErrCode::OutOfMemory);
        assert!(memory.get(&who, &Value::U8(1)).unwrap().is_nil());
    }

    #[test]
    fn unsupported_address_is_memory_error() {
        let mut raw = [0u8; 21];
        raw[0] = 2;
        let mut memory = map(24);
        let err = init(
            EffectMode::Edit,
            &mut memory,
            Address::from(raw),
            Value::U8(0),
        )
        .unwrap_err();
        assert_eq!(err.0, ItrErrCode::MemoryError);
    }
}
