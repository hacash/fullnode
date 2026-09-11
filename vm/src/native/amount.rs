//! Amount precision natives: the lossless siblings of `hac_to_zhu` / `hac_to_mei`.
//!
//! `Amount` is `value × 10^(unit−248)` HAC, so a transfer can carry digits below
//! the unit a contract books in (a custody balance can even sit at unit 238). The
//! truncating converters (`hac_to_unit` / `hac_to_zhu` / `hac_to_mei`) drop those
//! digits; a contract that books the integer then
//! holds more in its balance than its ledger claims. These rows either refuse to
//! convert (`hac_to_*_checked`, reverting) or report the precision as a predicate
//! (`hac_is_exact_*`). "Exact" is a numeric property, not the wire unit field
//! (`10:239` is exactly 1 zhu even though its unit is finer):
//!   `hac_to_unit_checked(unit, x)` ok  <=>  `hac_is_exact_unit(unit, x)` true
//!   <=> `x.is_exact_unit(unit)`, i.e. `x` has no non-zero part below `unit`.
//!   (`ok` implies the predicate; the converse also needs the scaled value to
//!   fit `u128`, which the predicate alone does not test.)
//!   `hac_is_exact_zhu` / `hac_to_zhu_checked` are the unit-240 shortcuts,
//!   `hac_is_exact_mei` / `hac_to_mei_checked` the unit-248 ones (whole HAC).
//!   `unit_to_hac(unit, value)` is the writer at this scale (`mei_to_hac` /
//!   `zhu_to_hac` are its unit-fixed forms), and round-trips with `hac_to_unit`.

use super::*;

/// Packed argv of `(unit u8, hacash bytes)` shared by the unit-parameterized rows.
fn unit_and_amount(argv: Value, cty: NativeFunc) -> VmrtRes<(u8, Amount)> {
    let args = func_argv(argv, cty)?;
    let unit = func_u8(&args[0], cty, "unit")?;
    let buf = func_bytes(&args[1], cty, "hacash")?;
    Ok((unit, decode_exact(&buf, cty.name())?))
}

/// Packed argv of `(unit u8, value u128)` shared by `unit_to_hac`.
fn unit_and_u128(argv: Value, cty: NativeFunc) -> VmrtRes<(u8, u128)> {
    let args = func_argv(argv, cty)?;
    let unit = func_u8(&args[0], cty, "unit")?;
    let value = func_u128(&args[1], cty, "value")?;
    Ok((unit, value))
}

fn exact_unit(hacash: &Amount, unit: u8) -> VmrtRes<bool> {
    hacash
        .is_exact_unit(unit)
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))
}

/// `hac_to_unit_checked(unit, hacash)` — `to_unit_u128`, failing instead of
/// truncating when the amount has precision below `unit`.
pub(crate) fn hac_to_unit_checked(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::hac_to_unit_checked;
    let (unit, hacash) = unit_and_amount(argv, cty)?;
    let value = hacash
        .to_unit_u128_exact(unit)
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U128(value))
}

/// `hac_to_unit(unit, hacash)` — `to_unit_u128`, the truncating sibling of
/// `hac_to_unit_checked`: it floors whatever sits below `unit` (and returns 0
/// when the whole amount is below it) instead of reverting. Offered because
/// reading an amount at an arbitrary unit is legitimate when the caller owns
/// the rounding rule; a ledger that books the integer should not use it.
pub(crate) fn hac_to_unit(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::hac_to_unit;
    let (unit, hacash) = unit_and_amount(argv, cty)?;
    let value = hacash
        .to_unit_u128(unit)
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U128(value))
}

/// `unit_to_hac(unit, value)` — the unit-parameterized writer: `value` counted
/// at `unit` becomes Amount bytes. Inverse of `hac_to_unit` for values that fit
/// (`unit_to_hac` canonicalizes trailing zeros into a coarser unit, which
/// `hac_to_unit` at the same `unit` folds straight back out).
pub(crate) fn unit_to_hac(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::unit_to_hac;
    let (unit, value) = unit_and_u128(argv, cty)?;
    Ok(Value::Bytes(Amount::coin_u128(value, unit).encode()))
}

/// `hac_to_zhu_checked(hacash)` — the unit-240 shortcut, what a payable hook
/// should call instead of booking `hac_to_zhu`'s integer.
pub(crate) fn hac_to_zhu_checked(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_to_zhu_checked")?;
    let zhu = hacash
        .to_zhu_u128_exact()
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U128(zhu))
}

/// `hac_to_mei_checked(hacash)` — the unit-248 shortcut (whole HAC), the
/// lossless sibling of `hac_to_mei`: same `u64` result for HAC-aligned amounts,
/// but reverting instead of flooring the sub-HAC part (up to 10^8 zhu).
pub(crate) fn hac_to_mei_checked(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_to_mei_checked")?;
    let mei = hacash
        .to_unit_u64_exact(UNIT_MEI)
        .map_err(|e| ItrErr::new(NativeFuncError, &e.to_string()))?;
    Ok(Value::U64(mei))
}

/// `hac_is_exact_unit(unit, hacash)` — non-reverting precision test.
pub(crate) fn hac_is_exact_unit(_: NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::hac_is_exact_unit;
    let (unit, hacash) = unit_and_amount(argv, cty)?;
    Ok(Value::Bool(exact_unit(&hacash, unit)?))
}

/// `hac_is_exact_zhu(hacash)` — the unit-240 shortcut.
pub(crate) fn hac_is_exact_zhu(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_is_exact_zhu")?;
    Ok(Value::Bool(exact_unit(&hacash, UNIT_ZHU)?))
}

/// `hac_is_exact_mei(hacash)` — the unit-248 shortcut (whole HAC), the
/// predicate twin of `hac_to_mei_checked`: true when the amount carries no
/// part below 1 HAC, i.e. when the u64 count is lossless.
pub(crate) fn hac_is_exact_mei(_: NativeFnEnv<'_>, buf: &[u8]) -> VmrtRes<Value> {
    let hacash: Amount = decode_exact(buf, "hac_is_exact_mei")?;
    Ok(Value::Bool(exact_unit(&hacash, UNIT_MEI)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::NativeArgvPack;

    fn call_env(f: impl FnOnce(NativeFnEnv<'_>) -> VmrtRes<Value>) -> VmrtRes<Value> {
        let cap = SpaceCap::new(0);
        f(NativeFnEnv::new(&cap))
    }

    fn call_concat(cty: NativeFunc, buf: &[u8]) -> VmrtRes<Value> {
        call_env(|env| NativeFunc::call(env, cty as u8, buf).map(|(v, _)| v))
    }

    fn call_packed(cty: NativeFunc, args: Vec<Value>) -> VmrtRes<Value> {
        let argv = Value::pack_call_args(args).unwrap();
        call_env(|env| NativeFunc::call_packed(env, cty as u8, argv).map(|(v, _)| v))
    }

    fn is_native_err<T: std::fmt::Debug>(r: VmrtRes<T>) -> bool {
        matches!(r, Err(ItrErr(NativeFuncError, _)))
    }

    #[test]
    fn catalog_rows_and_pack_are_stable() {
        assert_eq!(NativeFunc::hac_to_mei as u8, 51);
        assert_eq!(NativeFunc::hac_to_mei_checked as u8, 52);
        assert_eq!(NativeFunc::hac_to_zhu as u8, 53);
        assert_eq!(NativeFunc::hac_to_zhu_checked as u8, 54);
        assert_eq!(NativeFunc::hac_to_unit as u8, 55);
        assert_eq!(NativeFunc::hac_to_unit_checked as u8, 56);
        assert_eq!(NativeFunc::hac_is_exact_mei as u8, 57);
        assert_eq!(NativeFunc::hac_is_exact_zhu as u8, 58);
        assert_eq!(NativeFunc::hac_is_exact_unit as u8, 59);
        assert_eq!(NativeFunc::mei_to_hac as u8, 60);
        assert_eq!(NativeFunc::zhu_to_hac as u8, 61);
        assert_eq!(NativeFunc::unit_to_hac as u8, 62);
        for (cty, pack) in [
            (NativeFunc::hac_to_zhu_checked, NativeArgvPack::Concat),
            (NativeFunc::hac_is_exact_zhu, NativeArgvPack::Concat),
            (NativeFunc::hac_to_unit_checked, NativeArgvPack::Packed),
            (NativeFunc::hac_is_exact_unit, NativeArgvPack::Packed),
            (NativeFunc::hac_to_mei_checked, NativeArgvPack::Concat),
            (NativeFunc::hac_to_unit, NativeArgvPack::Packed),
            (NativeFunc::unit_to_hac, NativeArgvPack::Packed),
            (NativeFunc::hac_is_exact_mei, NativeArgvPack::Concat),
        ] {
            assert_eq!(cty.argv_pack_of(), pack, "{}", cty.name());
            assert_eq!(NativeFunc::argv_pack(cty as u8).unwrap(), pack);
        }
        // name resolution is what makes the rows callable from ircode source
        for (name, idx) in [
            ("hac_to_zhu_checked", 54u8),
            ("hac_to_unit_checked", 56),
            ("hac_is_exact_zhu", 58),
            ("hac_is_exact_unit", 59),
            ("hac_to_mei_checked", 52),
            ("hac_to_unit", 55),
            ("unit_to_hac", 62),
            ("hac_is_exact_mei", 57),
        ] {
            assert_eq!(NativeFunc::from_name(name).unwrap().0, idx);
        }
    }

    #[test]
    fn zhu_checked_keeps_exact_amounts_and_rejects_sub_zhu() {
        let one_hac = Amount::mei(1);
        assert_eq!(
            call_concat(NativeFunc::hac_to_zhu_checked, &one_hac.encode()).unwrap(),
            Value::U128(100_000_000)
        );
        // 10:239 is exactly 1 zhu even though its unit is finer
        let scaled = Amount::coin_u128(10, UNIT_ZHU - 1);
        assert_eq!(
            call_concat(NativeFunc::hac_to_zhu_checked, &scaled.encode()).unwrap(),
            Value::U128(1)
        );
        // 1 HAC + 1:239 — 10⁻⁹ HAC the truncating row would silently eat
        let leak = Amount::from("1.000000001").unwrap();
        assert_eq!(
            call_concat(NativeFunc::hac_to_zhu, &leak.encode()).unwrap(),
            Value::U128(100_000_000)
        );
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_zhu_checked,
            &leak.encode()
        )));
        // non-Amount bytes and negative amounts are errors, not silent passes
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_zhu_checked,
            &[1, 2, 3]
        )));
        let neg = Amount::from("-1:240").unwrap();
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_zhu_checked,
            &neg.encode()
        )));
    }

    #[test]
    fn unit_checked_follows_the_requested_unit() {
        // the real custody balance shape: exact at its own unit, sub-zhu by 52 parts
        let balance = Amount::from("10008568472493552:238").unwrap();
        assert_eq!(
            call_packed(
                NativeFunc::hac_to_unit_checked,
                vec![Value::U8(UNIT_238), Value::bytes(balance.encode())]
            )
            .unwrap(),
            Value::U128(10008568472493552)
        );
        assert!(is_native_err(call_packed(
            NativeFunc::hac_to_unit_checked,
            vec![Value::U8(UNIT_ZHU), Value::bytes(balance.encode())]
        )));
        assert_eq!(balance.to_zhu_u128().unwrap(), 100_085_684_724_935);
        // a finer target is always exact, and a u16 literal narrows to the u8 param
        assert_eq!(
            call_packed(
                NativeFunc::hac_to_unit_checked,
                vec![Value::U16(UNIT_SHUO as u16), Value::bytes(balance.encode())]
            )
            .unwrap(),
            Value::U128(10008568472493552 * 10u128.pow(6))
        );
        // argv shape/type errors stay native errors, not casts
        assert!(is_native_err(call_packed(
            NativeFunc::hac_to_unit_checked,
            vec![Value::bytes(balance.encode()), Value::U8(240)]
        )));
        assert!(is_native_err(call_packed(
            NativeFunc::hac_to_unit_checked,
            vec![Value::bytes(balance.encode())]
        )));
        assert!(is_native_err(call_packed(
            NativeFunc::hac_to_unit_checked,
            vec![Value::U16(256), Value::bytes(balance.encode())]
        )));
    }

    #[test]
    fn zhu_check_is_the_predicate_of_the_checked_row() {
        for amount in [
            Amount::mei(1),
            Amount::zhu(7),
            Amount::zero(),
            Amount::coin_u128(10, UNIT_ZHU - 1),
            Amount::from("1.000000001").unwrap(),
            Amount::from("10008568472493552:238").unwrap(),
            Amount::coin_u128(1, 0),
        ] {
            let buf = amount.encode();
            let checked = call_concat(NativeFunc::hac_to_zhu_checked, &buf);
            let bool_check = call_concat(NativeFunc::hac_is_exact_zhu, &buf);
            assert_eq!(
                bool_check.unwrap(),
                Value::Bool(checked.is_ok()),
                "amount={amount:?}"
            );
            // unit 240 rows agree with the unit-parameterized ones
            let packed_check = call_packed(
                NativeFunc::hac_is_exact_unit,
                vec![Value::U8(UNIT_ZHU), Value::bytes(buf.clone())],
            );
            assert_eq!(packed_check.unwrap(), Value::Bool(checked.is_ok()));
        }
        // every amount is exact at its own unit, and unit 0 is exact by definition
        let sub = Amount::coin_u128(1, UNIT_ZHU - 1);
        assert_eq!(
            call_packed(
                NativeFunc::hac_is_exact_unit,
                vec![Value::U8(UNIT_ZHU - 1), Value::bytes(sub.encode())]
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            call_packed(
                NativeFunc::hac_is_exact_unit,
                vec![Value::U8(0), Value::bytes(sub.encode())]
            )
            .unwrap(),
            Value::Bool(true)
        );
        // a unit beyond the amount's own needs the digits to be divisible by 10^k
        assert_eq!(
            call_packed(
                NativeFunc::hac_is_exact_unit,
                vec![Value::U8(UNIT_MEI), Value::bytes(Amount::mei(1).encode())]
            )
            .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            call_packed(
                NativeFunc::hac_is_exact_unit,
                vec![
                    Value::U8(UNIT_MEI),
                    Value::bytes(Amount::coin_u128(1, UNIT_MEI - 1).encode())
                ]
            )
            .unwrap(),
            Value::Bool(false)
        );
    }

    #[test]
    fn mei_checked_is_the_lossless_sibling_of_hac_to_mei() {
        // whole-HAC amounts: both rows agree
        for amount in [Amount::zero(), Amount::mei(1), Amount::zhu(100_000_000)] {
            let buf = amount.encode();
            let checked = call_concat(NativeFunc::hac_to_mei_checked, &buf).unwrap();
            assert_eq!(checked, call_concat(NativeFunc::hac_to_mei, &buf).unwrap());
            assert_eq!(checked.ty(), ValueTy::U64, "row must return u64");
        }
        assert_eq!(
            call_concat(NativeFunc::hac_to_mei_checked, &Amount::mei(7).encode()).unwrap(),
            Value::U64(7)
        );

        // 1 HAC + 1 zhu: `hac_to_mei` floors it, `hac_to_mei_checked` reverts.
        // This is the whole point of the row: the floored row books less than
        // the balance it just received.
        let leak = Amount::from("1.00000001").unwrap();
        assert_eq!(
            call_concat(NativeFunc::hac_to_mei, &leak.encode()).unwrap(),
            Value::U64(1)
        );
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_mei_checked,
            &leak.encode()
        )));
        // the predicate twin of the same amount agrees with the checked row
        assert_eq!(
            call_packed(
                NativeFunc::hac_is_exact_unit,
                vec![Value::U8(UNIT_MEI), Value::bytes(leak.encode())]
            )
            .unwrap(),
            Value::Bool(false)
        );

        // a u64 count that overflows is an error, not a wrapped number
        let over = Amount::coin_u128(u64::MAX as u128 + 1, UNIT_MEI);
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_mei_checked,
            &over.encode()
        )));
        // non-Amount bytes and its own row arg pack are errors too
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_mei_checked,
            &[1, 2, 3]
        )));
        assert!(NativeFunc::call_packed(
            NativeFnEnv::new(&SpaceCap::new(0)),
            NativeFunc::hac_to_mei_checked as u8,
            Value::bytes(Amount::mei(1).encode()),
        )
        .is_err());
    }

    #[test]
    fn mei_check_is_the_predicate_of_the_checked_row() {
        for amount in [
            Amount::zero(),
            Amount::mei(1),
            Amount::zhu(100_000_000),
            Amount::from("10008568472493552:238").unwrap(),
            Amount::from("1.00000001").unwrap(),
            Amount::zhu(1),
            Amount::coin_u128(1, 0),
        ] {
            let buf = amount.encode();
            let checked = call_concat(NativeFunc::hac_to_mei_checked, &buf);
            let by_shortcut = call_concat(NativeFunc::hac_is_exact_mei, &buf).unwrap();
            assert_eq!(
                by_shortcut,
                Value::Bool(checked.is_ok()),
                "amount={amount:?}"
            );
            // shortcut and the unit-parameterized row agree
            assert_eq!(
                by_shortcut,
                call_packed(
                    NativeFunc::hac_is_exact_unit,
                    vec![Value::U8(UNIT_MEI), Value::bytes(buf)]
                )
                .unwrap(),
                "amount={amount:?}"
            );
        }
        // non-Amount bytes and negative amounts are errors, not silent passes
        assert!(is_native_err(call_concat(NativeFunc::hac_is_exact_mei, &[1, 2, 3])));
        assert!(is_native_err(call_concat(
            NativeFunc::hac_is_exact_mei,
            &Amount::from("-1:240").unwrap().encode()
        )));
        // it is a Concat row: the packed entry point must refuse it
        assert!(NativeFunc::call_packed(
            NativeFnEnv::new(&SpaceCap::new(0)),
            NativeFunc::hac_is_exact_mei as u8,
            Value::bytes(Amount::mei(1).encode()),
        )
        .is_err());
    }

    /// The truncating reader: same floors as the fixed-unit rows, no revert.
    #[test]
    fn hac_to_unit_is_the_truncating_sibling() {
        // agrees with the unit-fixed truncating rows
        for amount in [
            Amount::mei(1),
            Amount::from("1.00000001").unwrap(),
            Amount::from("10008568472493552:238").unwrap(),
            Amount::zhu(900_000_000_000),
        ] {
            let buf = amount.encode();
            assert_eq!(
                call_packed(
                    NativeFunc::hac_to_unit,
                    vec![Value::U8(UNIT_ZHU), Value::bytes(buf.clone())]
                )
                .unwrap(),
                call_concat(NativeFunc::hac_to_zhu, &buf).unwrap(),
                "amount={amount:?} @zhu"
            );
            assert_eq!(
                call_packed(
                    NativeFunc::hac_to_unit,
                    vec![Value::U8(UNIT_MEI), Value::bytes(buf.clone())]
                )
                .unwrap()
                .extract_u128()
                .unwrap(),
                call_concat(NativeFunc::hac_to_mei, &buf)
                    .unwrap()
                    .extract_u128()
                    .unwrap(),
                "amount={amount:?} @mei"
            );
        }
        // 1 zhu + 10⁻⁹ HAC: the truncating rows floor it, the checked row reverts
        let leak = Amount::from("1.000000001").unwrap();
        assert_eq!(
            call_packed(
                NativeFunc::hac_to_unit,
                vec![Value::U8(UNIT_ZHU), Value::bytes(leak.encode())]
            )
            .unwrap(),
            Value::U128(100_000_000)
        );
        assert!(is_native_err(call_packed(
            NativeFunc::hac_to_unit_checked,
            vec![Value::U8(UNIT_ZHU), Value::bytes(leak.encode())]
        )));
        // fully below the unit reads as 0 (what the checked row reverts on)
        let dust = Amount::coin_u128(1, 0).encode();
        assert_eq!(
            call_packed(
                NativeFunc::hac_to_unit,
                vec![Value::U8(UNIT_ZHU), Value::bytes(dust.clone())]
            )
            .unwrap(),
            Value::U128(0)
        );
        assert!(is_native_err(call_packed(
            NativeFunc::hac_to_unit_checked,
            vec![Value::U8(UNIT_ZHU), Value::bytes(dust)]
        )));
        // Packed row refuses the concat entry point
        assert!(call_concat(NativeFunc::hac_to_unit, &Amount::mei(1).encode()).is_err());
    }

    /// The writer round-trips with both readers, and matches the fixed-unit writers.
    #[test]
    fn unit_to_hac_round_trips_with_hac_to_unit() {
        for unit in [UNIT_MIAO, UNIT_AI, UNIT_SHUO, UNIT_238, UNIT_ZHU, UNIT_244, UNIT_MEI] {
            for value in [0u128, 1, 7, 1_000_000_000, u64::MAX as u128, 1u128 << 100] {
                let bytes = call_packed(
                    NativeFunc::unit_to_hac,
                    vec![Value::U8(unit), Value::U128(value)],
                )
                .unwrap();
                let Value::Bytes(buf) = &bytes else {
                    panic!("unit_to_hac must return Amount bytes");
                };
                let read = |cty| {
                    call_packed(
                        cty,
                        vec![Value::U8(unit), Value::bytes(buf.clone())],
                    )
                    .unwrap()
                };
                assert_eq!(read(NativeFunc::hac_to_unit), Value::U128(value), "unit={unit}");
                assert_eq!(
                    read(NativeFunc::hac_to_unit_checked),
                    Value::U128(value),
                    "unit={unit} must be exact"
                );
                assert_eq!(read(NativeFunc::hac_is_exact_unit), Value::Bool(true));
            }
        }
        // the unit-fixed writers are the same function at their unit
        for value in [0u128, 1, 12345, 10u128.pow(30)] {
            let via_unit = call_packed(
                NativeFunc::unit_to_hac,
                vec![Value::U8(UNIT_ZHU), Value::U128(value)],
            )
            .unwrap();
            assert_eq!(via_unit, call_packed(NativeFunc::zhu_to_hac, vec![Value::U128(value)]).unwrap());
            // mei_to_hac is u64-typed, so it only covers values that fit
            let Ok(mei) = u64::try_from(value) else {
                continue;
            };
            let via_unit_mei = call_packed(
                NativeFunc::unit_to_hac,
                vec![Value::U8(UNIT_MEI), Value::U128(value)],
            )
            .unwrap();
            assert_eq!(
                via_unit_mei,
                call_packed(NativeFunc::mei_to_hac, vec![Value::U64(mei)]).unwrap()
            );
        }
        // a HAC count above u64 reads back through the u64 row as an error
        let huge = call_packed(
            NativeFunc::unit_to_hac,
            vec![Value::U8(UNIT_MEI), Value::U128(u64::MAX as u128 + 1)],
        )
        .unwrap();
        let Value::Bytes(huge) = &huge else {
            panic!("unit_to_hac must return Amount bytes");
        };
        assert!(is_native_err(call_concat(
            NativeFunc::hac_to_mei_checked,
            huge
        )));
        // Packed row refuses the concat entry point
        assert!(call_concat(NativeFunc::unit_to_hac, &[0]).is_err());
    }
}
