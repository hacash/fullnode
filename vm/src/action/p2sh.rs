//! `P2SHScriptProve` (kind 46) top-level action + P2SH lock-script hashing helpers.
//!
//! Ported from fullnodedev `vm/src/action/p2sh.rs`. Adaptations:
//! - `combi_struct!`/`combi_list!` macros (dev field crate) -> plain structs with manual
//!   `Encode`/`Decode` + `ListW1<PosiHash>` alias.
//! - `ContractAddressW1` -> `ContractAddrListW1` (= `ListW1<ContractAddress>`, defined in
//!   `vm/src/contract/mod.rs`).
//! - `Address::create_scriptmh(payload20)` is constructed inline with dev's
//!   scriptmh version byte `5`.
//! - `sha3`/`ripemd160` free fns (dev sys) -> local helpers over the `sha3`/`ripemd` crates.
//! - `.serialize()` on `Hash`/`ContractAddressW1` -> `.encode()` / `.as_bytes()`.

use std::sync::Arc;

use base::{ActScope, ActionRef, Context, ExecFrom, P2sh};
use field::{Address, BytesW2, Decode, Encode, Hash, Reader, Uint1, Uint2};
use ripemd::{Digest, Ripemd160};
use sha3::Sha3_256;
use sys::{Rerr, Ret, errf};

use crate::contract::ContractAddrListW1;
use crate::machine::peek_vm_runtime_limits;
use crate::rt::{CodeConf, GasExtra, SpaceCap};

base::impl_fields_to_json!(PosiHash { posi, hash });
// ================================ PosiHash / MerkelStuffs ================================

/// One Merkle proof step: sibling hash + left/right position.
/// `posi==0` -> sibling on the LEFT, `posi==1` -> sibling on the RIGHT.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PosiHash {
    pub posi: Uint1,
    pub hash: Hash,
}

impl Encode for PosiHash {
    fn size(&self) -> usize {
        self.posi.size() + self.hash.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.posi.encode_to(out);
        self.hash.encode_to(out);
    }
}

impl Decode for PosiHash {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let posi = r.read()?;
        let hash = r.read()?;
        Ok((Self { posi, hash }, r.used()))
    }
}

/// Merkle proof path (list of `PosiHash` siblings).
pub type MerkelStuffs = field::ListW1<PosiHash>;

// ================================ UnlockScript / ScriptmhCalc ================================

pub struct UnlockScript {
    codeconf: u8,
    stuff: Vec<u8>,
    witness: Vec<u8>,
}

/// Result of `scriptmh` address derivation for a P2SH lock script.
///
/// Hashing rules (same as `P2SHScriptProve::get_merkel()`):
/// - Leaf: `sha3("p2sh_leaf_" || libs || codeconf || lockbox)`
/// - Branch i: `sha3("p2sh_branch_" || left || right)` where `(left,right)` is decided by `posi`.
/// - Address: `Address` with scriptmh version byte `5` carrying `ripemd160(root_sha3)`.
#[derive(Debug, Clone)]
pub struct ScriptmhCalc {
    /// Final `SCRIPTMH` address (dev-compatible version byte `5`).
    pub address: Address,
    /// `ripemd160(root_sha3)` that becomes the address payload (20 bytes).
    pub payload20: [u8; 20],
    /// SHA3-256 chain. `sha3_path[0]` is the leaf hash, `sha3_path.last()` is the root hash.
    pub sha3_path: Vec<Hash>,
}

impl P2sh for UnlockScript {
    fn code_conf(&self) -> u8 {
        self.codeconf
    }
    fn code_stuff(&self) -> &[u8] {
        &self.stuff
    }
    fn witness(&self) -> &[u8] {
        &self.witness
    }
}

// ================================ P2shEntryPayload ================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct P2shEntryPayload {
    pub state_addr: Address,
    pub libs: ContractAddrListW1,
    pub codes: BytesW2,
}

impl P2shEntryPayload {
    pub fn parse(payload: &[u8]) -> Ret<Self> {
        let mut r = Reader::new(payload);
        let state_addr: Address = r.read()?;
        must_scriptmh(&state_addr)?;
        let libs: ContractAddrListW1 = r.read()?;
        let codes = BytesW2::from(payload[r.used()..].to_vec())?;
        Ok(Self {
            state_addr,
            libs,
            codes,
        })
    }

    pub fn build(state_addr: Address, libs: ContractAddrListW1, codes: BytesW2) -> Ret<Vec<u8>> {
        must_scriptmh(&state_addr)?;
        let mut out = state_addr.encode();
        out.extend(libs.encode());
        out.extend_from_slice(codes.as_vec());
        Ok(out)
    }

    pub fn verify_unlock_inputs(
        &self,
        block_height: u64,
        gst: &GasExtra,
        cap: &SpaceCap,
        codeconf: CodeConf,
        witness: &BytesW2,
        registry: &dyn base::ExecutionServices,
    ) -> Ret<()> {
        P2SHScriptProve::verify_unlock_inputs(
            block_height,
            gst,
            cap,
            &self.libs,
            codeconf,
            &self.codes,
            witness,
            registry,
        )
    }
}

// ================================ P2SHScriptProve ================================

#[derive(Debug, Clone, PartialEq, Eq, base::ActionCodec)]
pub struct P2SHScriptProve {
    pub kind: Uint2,
    // calc hash: script + calibs
    pub argvkey: BytesW2,            // unlock witness bytes (not executed)
    pub adrlibs: ContractAddrListW1, // lib address list for pure and codecall
    pub codeconf: Uint1,             // low 2 bits: CodeType, high 6 bits: reserved (must be 0)
    pub lockbox: BytesW2,            // verify bytecodes
    pub merkels: MerkelStuffs,
    pub marks: Fixed2, // zero
}

use field::Fixed2;

impl P2SHScriptProve {
    pub const KIND: u16 = 46;

    pub fn new() -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            argvkey: BytesW2::default(),
            adrlibs: ContractAddrListW1::default(),
            codeconf: Uint1::from(0),
            lockbox: BytesW2::default(),
            merkels: MerkelStuffs::default(),
            marks: Fixed2::default(),
        }
    }
}

impl Default for P2SHScriptProve {
    fn default() -> Self {
        Self::new()
    }
}

base::impl_action! {
    P2SHScriptProve {
        name: "p2sh_script_prove",
        scope: ActScope::TOP,
        min_tx_type: 3,
        extra9: |_: &P2SHScriptProve| false,
        req_sign: |_: &P2SHScriptProve| vec![],
        as_transfer_like: none,
        description: |_: &P2SHScriptProve| "Prove P2SH unlock script".to_owned(),
        execute: (self, ctx) {
        #[cfg(all(feature = "codec-only", not(feature = "full")))]
        {
            let _ = (self, ctx);
            crate::action::execution_disabled()
        }
        #[cfg(not(all(feature = "codec-only", not(feature = "full"))))]
        {
            p2sh_script_prove_execute(self, ctx)?;
            Ok(vec![])
        }
        }
    }
}

fn p2sh_script_prove_execute(this: &P2SHScriptProve, ctx: &mut dyn Context) -> Rerr {
    if ctx.exec_from() != ExecFrom::Top {
        return errf!(
            "P2SHScriptProve can only run in TOP context, got {}",
            ctx.exec_from()
        );
    }
    if !this.marks.is_zero() {
        return errf!("marks bytes format invalid");
    }
    let hei = ctx.env().block.height;
    let (_, cap) = peek_vm_runtime_limits(ctx, hei);
    let p2sh_count = ctx.p2sh_count();
    if p2sh_count >= cap.p2sh_set {
        return errf!("p2sh_set overflow (>={})", cap.p2sh_set);
    }
    P2SHScriptProve::verify_merkle_depth(&cap, this.merkels.as_list().len())?;
    let adr = this.get_merkel()?;
    let stuff = this.get_stuff_with_merkel(ctx, &adr)?;
    ctx.p2sh_set(adr, Box::new(stuff))?;
    // finish
    Ok(())
}

impl P2SHScriptProve {
    /// Compute the `SCRIPTMH` address from:
    /// - `adrlibs`: the contract libraries allowlist used by the P2SH lock script
    /// - `codeconf`: script code config byte
    /// - `lockbox`: the P2SH lock script bytecode (as it appears in this action field)
    /// - `merkels`: the Merkle proof path (siblings + left/right positions) used to commit
    ///   the lock script into a Merkle root.
    ///
    /// Notes for tooling authors:
    /// - `codeconf` is hashed as one raw byte.
    /// - lockbox is hashed as raw data bytes (`BytesW2::to_vec()`, without length prefix)
    ///   not as a custom encoding.
    /// - Each sibling `hash` is hashed as its raw 32 bytes.
    pub fn calc_scriptmh_from_lockbox(
        adrlibs: &ContractAddrListW1,
        codeconf: CodeConf,
        lockbox: &BytesW2,
        merkels: &MerkelStuffs,
    ) -> Ret<ScriptmhCalc> {
        let mut h = Hash::from(p2sh_sha3(
            vec![
                "p2sh_leaf_".as_bytes().to_vec(), // domain separator for safety
                adrlibs.encode(),
                vec![codeconf.raw()],
                lockbox.to_vec(),
            ]
            .concat(),
        ));
        let mut path = vec![h];
        for step in merkels.as_list().iter() {
            let posi = step.posi.uint();
            if posi > 1 {
                return errf!("p2sh Merkle position {} invalid, must be 0 or 1", posi);
            }
            let ch = h;
            if step.hash == ch {
                return errf!("p2sh Merkle self pair is not allowed");
            }
            // left or right: posi==0 means sibling on the left, posi==1 means sibling on the right.
            let pair = maybe!(posi == 0, [step.hash, ch], [ch, step.hash]);
            let mut buf: Vec<u8> = "p2sh_branch_".as_bytes().to_vec(); // domain separator for safety
            for a in pair.iter() {
                buf.extend_from_slice(a.as_bytes());
            }
            h = Hash::from(p2sh_sha3(buf));
            path.push(h);
        }
        let payload20 = p2sh_ripemd160(h.as_bytes());
        Ok(ScriptmhCalc {
            address: create_scriptmh_addr(payload20),
            payload20,
            sha3_path: path,
        })
    }

    fn verify_adrlibs(cap: &SpaceCap, adrlibs: &ContractAddrListW1) -> Ret<()> {
        let libs = adrlibs.as_list();
        if libs.len() > cap.library {
            return errf!("p2sh libs overflow (>={})", cap.library);
        }
        if !libs.iter().all(|a| a.is_contract()) {
            return errf!("contract libs invalid");
        }
        let mut libset = std::collections::HashSet::with_capacity(libs.len());
        for a in libs.iter() {
            if !libset.insert(*a) {
                return errf!("duplicate p2sh lib address '{}'", a.to_readable());
            }
        }
        Ok(())
    }

    pub fn verify_unlock_inputs(
        block_height: u64,
        gst: &GasExtra,
        cap: &SpaceCap,
        adrlibs: &ContractAddrListW1,
        codeconf: CodeConf,
        lockbox: &BytesW2,
        witness: &BytesW2,
        registry: &dyn base::ExecutionServices,
    ) -> Ret<()> {
        Self::verify_adrlibs(cap, adrlibs)?;
        Self::verify_lockbox_bytes(cap, lockbox.as_vec())?;
        crate::contract::convert_and_check(
            cap,
            gst,
            codeconf.code_type(),
            lockbox.as_vec(),
            block_height,
            registry,
        )
        .map_err(sys::Error::from)?;
        Self::verify_witness_bytes(cap, witness.as_vec())?;
        Ok(())
    }

    fn verify_lockbox_bytes(cap: &SpaceCap, lockbox: &[u8]) -> Ret<()> {
        if lockbox.len() > cap.p2sh_lockbox_size_max {
            return errf!(
                "p2sh lockbox bytes too long (>={})",
                cap.p2sh_lockbox_size_max
            );
        }
        Ok(())
    }

    pub fn verify_merkle_depth(cap: &SpaceCap, merkle_depth: usize) -> Ret<()> {
        if merkle_depth > cap.p2sh_merkle_depth_max {
            return errf!(
                "p2sh merkle depth overflow (>={})",
                cap.p2sh_merkle_depth_max
            );
        }
        Ok(())
    }

    fn verify_witness_bytes(cap: &SpaceCap, witness: &[u8]) -> Ret<()> {
        if witness.len() > cap.value_size {
            return errf!("p2sh witness bytes too long");
        }
        Ok(())
    }

    fn get_stuff_with_merkel(
        &self,
        ctx: &mut dyn Context,
        scriptmh: &Address,
    ) -> Ret<UnlockScript> {
        let hei = ctx.env().block.height;
        let (gst, cap) = peek_vm_runtime_limits(ctx, hei);
        let codeconf = CodeConf::parse(self.codeconf.uint()).map_err(sys::Error::from)?;
        Self::verify_unlock_inputs(
            hei,
            &gst,
            &cap,
            &self.adrlibs,
            codeconf,
            &self.lockbox,
            &self.argvkey,
            ctx.services().as_ref(),
        )?;
        let lockbox = self.lockbox.to_vec();
        let witness = self.argvkey.to_vec();
        // ok
        let merkel = scriptmh.as_bytes().to_vec();
        let libs = self.adrlibs.encode();
        let mut stuff = Vec::with_capacity(merkel.len() + libs.len() + lockbox.len());
        stuff.extend_from_slice(&merkel);
        stuff.extend_from_slice(&libs);
        stuff.extend_from_slice(&lockbox);
        Ok(UnlockScript {
            codeconf: codeconf.raw(),
            stuff,
            witness,
        })
    }

    fn get_merkel(&self) -> Ret<Address> {
        let codeconf = CodeConf::parse(self.codeconf.uint()).map_err(sys::Error::from)?;
        Ok(
            Self::calc_scriptmh_from_lockbox(
                &self.adrlibs,
                codeconf,
                &self.lockbox,
                &self.merkels,
            )?
            .address,
        )
    }
}

// ================================ hashing helpers ================================

pub(crate) fn p2sh_sha3(data: impl AsRef<[u8]>) -> [u8; 32] {
    let mut hasher = Sha3_256::new();
    hasher.update(data.as_ref());
    hasher.finalize().into()
}

pub(crate) fn p2sh_ripemd160(data: impl AsRef<[u8]>) -> [u8; 20] {
    let mut hasher = Ripemd160::new();
    hasher.update(data.as_ref());
    hasher.finalize().into()
}

/// Construct a dev-compatible scriptmh-version (`5`) address carrying the given 20-byte payload.
pub(crate) fn create_scriptmh_addr(payload20: [u8; 20]) -> Address {
    let mut raw = [0u8; Address::SIZE];
    raw[0] = 5;
    raw[1..].copy_from_slice(&payload20);
    Address::from(raw)
}

fn must_scriptmh(addr: &Address) -> Ret<()> {
    if !addr.is_scriptmh() {
        return errf!("address {} is not scriptmh type", addr.to_readable());
    }
    Ok(())
}

// ================================ decoder ================================

pub fn create_p2sh_script_prove(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let (action, used) = P2SHScriptProve::decode(buf)?;
    Ok((Arc::new(action), used))
}
