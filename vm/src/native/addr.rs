use crate::rt::{NativeFunc, VmrtRes};
use crate::value::Value;

use super::func_address;

/// `address_version(addr | bytes21) -> u8`: raw protocol version byte of a supported
/// address (`PRIVAKEY` 0 / `CONTRACT` 1 / `SCRIPTMH` 5). Input goes through
/// `extract_address`, so unsupported versions and non-21-byte values fail instead of
/// leaking an unchecked version byte.
pub(crate) fn address_version(_: super::NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::address_version;
    let addr = func_address(&packed_single(argv, cty)?, cty, "address")?;
    Ok(Value::U8(addr.version()))
}

/// `is_privkey_unknown(addr | bytes21) -> Bool`: PRIVAKEY address whose 21-byte value is
/// below `u32::MAX` — a system-reserved address with an unknown private key (BLACKHOLE,
/// TEX settlement, ...). Signer lists must reject these or `gov()`/`release()` freezes.
pub(crate) fn is_privkey_unknown(_: super::NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::is_privkey_unknown;
    let addr = func_address(&packed_single(argv, cty)?, cty, "address")?;
    Ok(Value::Bool(addr.is_privkey_unknown()))
}

/// `is_privkey_not_unknown(addr | bytes21) -> Bool`: the full signer-eligibility
/// predicate — PRIVAKEY version and not system-reserved. Matches the host-side
/// `RequiredSigners::validate_against` checks (version + unknown-key), so a contract can
/// pre-validate a signer with one call instead of hand-written byte probes, and failure
/// is a visible `false` rather than a later, unrecoverable sigset load error.
pub(crate) fn is_privkey_not_unknown(_: super::NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
    let cty = NativeFunc::is_privkey_not_unknown;
    let addr = func_address(&packed_single(argv, cty)?, cty, "address")?;
    Ok(Value::Bool(addr.is_privkey() && !addr.is_privkey_unknown()))
}

macro_rules! version_predicate {
    ($fn_name:ident, $cty:ident, $pred:ident, $doc:expr) => {
        #[doc = $doc]
        pub(crate) fn $fn_name(_: super::NativeFnEnv<'_>, argv: Value) -> VmrtRes<Value> {
            let cty = NativeFunc::$cty;
            let addr = func_address(&packed_single(argv, cty)?, cty, "address")?;
            Ok(Value::Bool(addr.$pred()))
        }
    };
}

version_predicate! {
    is_privkey, is_privkey, is_privkey,
    "`is_privkey(addr | bytes21) -> Bool`: version byte is PRIVAKEY (0), system-reserved or not."
}
version_predicate! {
    is_contract, is_contract, is_contract,
    "`is_contract(addr | bytes21) -> Bool`: version byte is CONTRACT (1)."
}
version_predicate! {
    is_scriptmh, is_scriptmh, is_scriptmh,
    "`is_scriptmh(addr | bytes21) -> Bool`: version byte is SCRIPTMH (5)."
}

fn packed_single(argv: Value, cty: NativeFunc) -> VmrtRes<Value> {
    debug_assert_eq!(cty.argv_len_of(), 1);
    let mut args = super::func_argv(argv, cty)?;
    Ok(args.pop().expect("catalog arity 1 is Raw"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rt::{ItrErr, ItrErrCode, NativeArgvPack, NativeFnEnv, SpaceCap};

    fn call_env(f: impl FnOnce(NativeFnEnv<'_>) -> VmrtRes<Value>) -> VmrtRes<Value> {
        let cap = SpaceCap::new(0);
        f(NativeFnEnv::new(&cap))
    }

    fn call(cty: NativeFunc, args: Vec<Value>) -> VmrtRes<Value> {
        debug_assert_eq!(cty.argv_pack_of(), NativeArgvPack::Packed);
        let argv = Value::pack_call_args(args).unwrap();
        call_env(|env| NativeFunc::call_packed(env, cty as u8, argv).map(|(v, _)| v))
    }

    fn addr_with_version(version: u8, tail: u8) -> field::Address {
        let mut raw = [0u8; 21];
        raw[0] = version;
        raw[20] = tail;
        field::Address::from(raw)
    }

    fn addr_value(v: field::Address) -> Value {
        Value::Address(v)
    }

    /// Same value as bytes: call boundary implicit conversion must land here identically.
    fn bytes21_value(v: field::Address) -> Value {
        Value::Bytes(v.as_bytes().to_vec())
    }

    #[test]
    fn catalog_rows_are_stable() {
        assert_eq!(NativeFunc::address_version as u8, 84);
        assert_eq!(NativeFunc::is_privkey_unknown as u8, 85);
        assert_eq!(NativeFunc::is_privkey_not_unknown as u8, 86);
        assert_eq!(NativeFunc::is_privkey as u8, 87);
        assert_eq!(NativeFunc::is_contract as u8, 88);
        assert_eq!(NativeFunc::is_scriptmh as u8, 89);
        for cty in [
            NativeFunc::address_version,
            NativeFunc::is_privkey_unknown,
            NativeFunc::is_privkey_not_unknown,
            NativeFunc::is_privkey,
            NativeFunc::is_contract,
            NativeFunc::is_scriptmh,
        ] {
            assert_eq!(cty.argv_len_of(), 1);
            assert_eq!(cty.argv_pack_of(), NativeArgvPack::Packed);
        }
    }

    /// Version predicates partition the three supported versions exactly:
    /// is_privkey(0), is_contract(1), is_scriptmh(5), and one version never
    /// satisfies two predicates.
    #[test]
    fn version_predicates_partition_supported_versions() {
        let cases: [(u8, NativeFunc, bool); 9] = [
            (field::Address::VERSION_PRIVAKEY, NativeFunc::is_privkey, true),
            (field::Address::VERSION_PRIVAKEY, NativeFunc::is_contract, false),
            (field::Address::VERSION_PRIVAKEY, NativeFunc::is_scriptmh, false),
            (field::Address::VERSION_CONTRACT, NativeFunc::is_privkey, false),
            (field::Address::VERSION_CONTRACT, NativeFunc::is_contract, true),
            (field::Address::VERSION_CONTRACT, NativeFunc::is_scriptmh, false),
            (field::Address::VERSION_SCRIPTMH, NativeFunc::is_privkey, false),
            (field::Address::VERSION_SCRIPTMH, NativeFunc::is_contract, false),
            (field::Address::VERSION_SCRIPTMH, NativeFunc::is_scriptmh, true),
        ];
        for (version, cty, want) in cases {
            let a = addr_with_version(version, 1);
            let r = call(cty, vec![addr_value(a)]).unwrap();
            assert_eq!(r, Value::Bool(want), "{} on version {}", cty.name(), version);
        }
    }

    #[test]
    fn version_predicates_accept_bytes21_implicitly() {
        let a = addr_with_version(field::Address::VERSION_CONTRACT, 2);
        let r = call(NativeFunc::is_contract, vec![bytes21_value(a)]).unwrap();
        assert_eq!(r, Value::Bool(true));
        let r = call(NativeFunc::is_privkey, vec![bytes21_value(a)]).unwrap();
        assert_eq!(r, Value::Bool(false));
    }

    #[test]
    fn address_version_reports_raw_version_byte() {
        let cases = [
            (field::Address::VERSION_PRIVAKEY, true),
            (field::Address::VERSION_CONTRACT, true),
            (field::Address::VERSION_SCRIPTMH, true),
        ];
        for (version, supported) in cases {
            assert!(addr_with_version(version, 1).is_supported() == supported);
            let r = call(NativeFunc::address_version, vec![addr_value(addr_with_version(version, 1))]).unwrap();
            assert_eq!(r, Value::U8(version), "version {version}");
        }
    }

    #[test]
    fn address_version_accepts_bytes21_and_rejects_other_shapes() {
        let addr = addr_with_version(field::Address::VERSION_CONTRACT, 7);
        let r = call(NativeFunc::address_version, vec![bytes21_value(addr)]).unwrap();
        assert_eq!(r, Value::U8(field::Address::VERSION_CONTRACT));

        // non-21-byte input cannot be an address
        let r = call(NativeFunc::address_version, vec![Value::Bytes(vec![0u8; 20])]);
        assert!(matches!(r, Err(ItrErr(ItrErrCode::NativeFuncError, _))));

        // unsupported version byte is rejected at the Address boundary
        let r = call(
            NativeFunc::address_version,
            vec![Value::Bytes(addr_with_version(0x02, 1).as_bytes().to_vec())],
        );
        assert!(matches!(r, Err(ItrErr(ItrErrCode::NativeFuncError, _))));
    }

    #[test]
    fn privkey_unknown_predicates() {
        // version 0, all-zero tail except last byte < 0xFF: unknown system key
        let system = addr_with_version(field::Address::VERSION_PRIVAKEY, 1);
        assert!(system.is_privkey_unknown());
        // BLACKHOLE (all zero) is also privkey-unknown
        let blackhole = field::Address::default();
        assert!(blackhole.is_privkey_unknown());

        for a in [system, blackhole] {
            let r = call(NativeFunc::is_privkey_unknown, vec![addr_value(a)]).unwrap();
            assert_eq!(r, Value::Bool(true), "{}", a.to_readable());
            let r = call(NativeFunc::is_privkey_not_unknown, vec![addr_value(a)]).unwrap();
            assert_eq!(r, Value::Bool(false), "{}", a.to_readable());
        }

        // a normal privkey address: big tail bytes push value above u32::MAX
        let normal = field::Address::from({
            let mut raw = [0u8; 21];
            raw[0] = field::Address::VERSION_PRIVAKEY;
            raw[1] = 0xAA;
            raw[20] = 0x99;
            raw
        });
        assert!(!normal.is_privkey_unknown());
        let r = call(NativeFunc::is_privkey_unknown, vec![addr_value(normal)]).unwrap();
        assert_eq!(r, Value::Bool(false));
        let r = call(NativeFunc::is_privkey_not_unknown, vec![addr_value(normal)]).unwrap();
        assert_eq!(r, Value::Bool(true));

        // contract / scriptmh versions are not privkey at all
        for v in [field::Address::VERSION_CONTRACT, field::Address::VERSION_SCRIPTMH] {
            let a = addr_with_version(v, 3);
            let r = call(NativeFunc::is_privkey_unknown, vec![addr_value(a)]).unwrap();
            assert_eq!(r, Value::Bool(false));
            let r = call(NativeFunc::is_privkey_not_unknown, vec![addr_value(a)]).unwrap();
            assert_eq!(r, Value::Bool(false));
        }
    }

    #[test]
    fn predicates_accept_bytes21_implicitly() {
        let normal = field::Address::from({
            let mut raw = [0u8; 21];
            raw[0] = field::Address::VERSION_PRIVAKEY;
            raw[2] = 0xBB;
            raw
        });
        let r = call(NativeFunc::is_privkey_not_unknown, vec![bytes21_value(normal)]).unwrap();
        assert_eq!(r, Value::Bool(true));
        let blackhole = field::Address::default();
        let r = call(NativeFunc::is_privkey_unknown, vec![bytes21_value(blackhole)]).unwrap();
        assert_eq!(r, Value::Bool(true));
    }
}
