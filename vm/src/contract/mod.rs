#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};
#[cfg(feature = "execute")]
use std::sync::Arc;

use field::{
    Decode, Encode, Fixed1, Fixed2, Fixed3, Fixed4, Hash, ListW1, ListW2, Reader, Uint1, Uint2,
    Uint4, Uint8,
};
use sys::Ret;

use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::ContractAddress;

#[cfg(feature = "execute")]
mod builder;
mod schema;

#[cfg(feature = "execute")]
pub use builder::{Abst, Contract, Func};

pub type ContractAddrListW1 = ListW1<ContractAddress>;
pub type ContractAbstCallList = ListW1<ContractAbstCall>;
pub type ContractUserFuncList = ListW2<ContractUserFunc>;
pub type ContractCalcFuncList = ListW2<ContractCalcFunc>;
pub type ContractAddrReplaceAtList = ListW1<ContractAddrReplaceAt>;

/// Calc func channel activation window: the channel is disabled in the
/// half-open height window `[CALCFUNC_DISABLE_FROM_HEIGHT,
/// CALCFUNC_DISABLE_UNTIL_HEIGHT)`. Test chains (height < 790,000) keep the
/// pre-launch window; the live network is currently inside the disabled
/// window, so deployment / upgrade keep rejecting calcfuncs. The channel
/// re-enables at 820,000; change these bounds only as part of a coordinated
/// consensus upgrade.
pub const CALCFUNC_DISABLE_FROM_HEIGHT: u64 = 790_000;
pub const CALCFUNC_DISABLE_UNTIL_HEIGHT: u64 = 820_000;

/// Whether calcfunc deployment / upgrade is allowed at `height`.
pub const fn calcfunc_enabled_at(height: u64) -> bool {
    height < CALCFUNC_DISABLE_FROM_HEIGHT || height >= CALCFUNC_DISABLE_UNTIL_HEIGHT
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractMeta {
    pub vrsn: Fixed1,
    pub revision: Uint2,
    pub mark: Fixed3,
    pub mext: Fixed4,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractAbstCall {
    pub sign: Fixed1,
    pub mark: Fixed2,
    pub fncnf: Fixed1,
    pub code_stuff: CodeStuff,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractUserFunc {
    pub sign: Fixed4,
    pub mark: Fixed3,
    pub fncnf: Fixed1,
    pub pmdf: FuncArgvTypes,
    pub code_stuff: CodeStuff,
}

/// A calc (RV64IMC) kernel attached to a contract.
///
/// The encoded container in `code_stuff.data` is the source of truth: its
/// header carries the ABI and feature flags and is what the verifier checks.
/// The remaining fields are wire-compat mirrors/reserved values that must
/// agree with it:
/// * `mark` mirrors the ABI and must be `1` (bytes -> bytes);
/// * `fncnf` must equal the container header's `fncnf` byte;
/// * `code_stuff.conf` is reserved for calc kernels and must be zero.
#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractCalcFunc {
    pub sign: Fixed4,
    pub mark: Fixed1,
    pub fncnf: Fixed1,
    pub code_stuff: CodeStuff,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractAddrReplaceAt {
    pub idx: Uint1,
    pub addr: ContractAddress,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractEdit {
    pub new_revision: Uint2,
    pub inherit_add: ContractAddrListW1,
    pub inherit_replace_at: ContractAddrReplaceAtList,
    pub library_add: ContractAddrListW1,
    pub library_replace_at: ContractAddrReplaceAtList,
    pub abstcalls: ContractAbstCallList,
    pub userfuncs: ContractUserFuncList,
    pub calcfuncs: ContractCalcFuncList,
}

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct ContractSto {
    pub metas: ContractMeta,
    pub inherit: ContractAddrListW1,
    pub library: ContractAddrListW1,
    pub abstcalls: ContractAbstCallList,
    pub userfuncs: ContractUserFuncList,
    pub calcfuncs: ContractCalcFuncList,
    pub morextend: Uint8,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContractEdition {
    pub revision: Uint2,
    pub raw_size: Uint4,
    pub hash: Hash,
}

impl Encode for ContractEdition {
    fn size(&self) -> usize {
        field::Encode::size(&self.revision)
            + field::Encode::size(&self.raw_size)
            + field::Encode::size(&self.hash)
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.revision.encode_to(out);
        self.raw_size.encode_to(out);
        self.hash.encode_to(out);
    }
}

impl Decode for ContractEdition {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let revision = r.read()?;
        let raw_size = r.read()?;
        let hash = r.read()?;
        Ok((
            Self {
                revision,
                raw_size,
                hash,
            },
            r.used(),
        ))
    }
}

#[cfg(feature = "execute")]
#[derive(Default)]
pub struct ContractObj {
    pub sto: ContractSto,
    pub abstfns: HashMap<AbstCall, Arc<FnObj>>,
    pub userfns: HashMap<FnSign, Arc<FnObj>>,
    /// Verified calc kernels, indexed by their `calcfuncs` list position.
    pub calcfns: Vec<Arc<crate::calc::CalcFnObj>>,
    pub edition: ContractEdition,
}

impl ContractAddrReplaceAt {
    fn idx_usize(&self) -> usize {
        self.idx.uint() as usize
    }
}

impl ContractMeta {
    fn check(&self, _height: u64) -> VmrtErr {
        if !self.vrsn.is_zero() || !self.mark.is_zero() || !self.mext.is_zero() {
            return itr_err_fmt!(ContractError, "contract format invalid");
        }
        Ok(())
    }
}

impl ContractAbstCall {
    fn check(&self, _height: u64) -> VmrtErr {
        if !self.mark.is_zero() || !self.fncnf.is_zero() {
            return itr_err_fmt!(ContractError, "contract ContractAbstCall format invalid");
        }
        Ok(())
    }
}

impl ContractUserFunc {
    fn check(&self, _height: u64) -> VmrtErr {
        if !self.mark.is_zero() {
            return itr_err_fmt!(ContractError, "contract ContractUserFunc format invalid");
        }
        let known = FnConf::External as u8;
        if self.fncnf[0] & !known != 0 {
            return itr_err_fmt!(ContractError, "contract ContractUserFunc format invalid");
        }
        Ok(())
    }
}

/// Verify the wire-compat mirrors on a calc func against its decoded
/// container. This is the single source of truth for the ABI / reserved-conf
/// / fncnf rules, shared by deploy-time `check_content` and load-time
/// `into_obj`.
#[cfg(feature = "execute")]
fn validate_calc_func(
    f: &ContractCalcFunc,
    c: &crate::calc::container::CalcContainer,
) -> VmrtErr {
    if f.mark[0] != 1 {
        return itr_err_fmt!(ContractError, "calcfunc mark (abi) must be 1");
    }
    if f.code_stuff.conf.uint() != 0 {
        return itr_err_fmt!(ContractError, "calcfunc code conf reserved, must be zero");
    }
    if c.fncnf != f.fncnf[0] {
        return itr_err_fmt!(ContractError, "calcfunc fncnf mismatch with container");
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn verify_code_stuff(
    cap: &SpaceCap,
    gas: &GasExtra,
    code_stuff: &CodeStuff,
    height: u64,
    registry: &dyn base::ExecutionServices,
) -> VmrtErr {
    let code_pkg = CodePkg::try_from(code_stuff)?;
    convert_and_check(
        cap,
        gas,
        code_pkg.code_type()?,
        &code_pkg.data,
        height,
        registry,
    )?;
    Ok(())
}

#[cfg(feature = "execute")]
pub fn convert_and_check(
    cap: &SpaceCap,
    gas: &GasExtra,
    ctype: CodeType,
    codes: &[u8],
    _height: u64,
    registry: &dyn base::ExecutionServices,
) -> VmrtRes<Vec<u8>> {
    let bytecodes = match ctype {
        CodeType::IRNode => crate::ir::runtime_irs_to_exec_bytecodes(codes, gas)?,
        CodeType::Bytecode => codes.to_vec(),
    };
    if bytecodes.len() > cap.function_size {
        return itr_err_code!(CodeTooLong);
    }
    verify_bytecodes_for_cap(&bytecodes, cap.value_size, registry)
}

fn list_replace<T>(list: &mut Vec<T>, idx: usize, value: T, err: ItrErrCode) -> VmrtErr {
    if idx >= list.len() {
        return Err(ItrErr::code(err));
    }
    list[idx] = value;
    Ok(())
}

fn append_checked<T>(dst: &mut Vec<T>, src: &[T], err: ItrErrCode) -> VmrtErr
where
    T: Clone,
{
    dst.try_reserve(src.len())
        .map_err(|e| ItrErr::new(err, &e.to_string()))?;
    dst.extend_from_slice(src);
    Ok(())
}

fn merge_abst_funcs(dst: &mut ContractAbstCallList, src: &ContractAbstCallList) -> VmrtRes<bool> {
    let mut edit = false;
    for func in src.as_list() {
        let mut replaced = false;
        for item in dst.as_mut() {
            if item.sign == func.sign {
                *item = func.clone();
                replaced = true;
                edit = true;
                break;
            }
        }
        if !replaced {
            dst.push(func.clone()).map_ire(ContractUpgradeErr)?;
        }
    }
    Ok(edit)
}

fn merge_calc_funcs(dst: &mut ContractCalcFuncList, src: &ContractCalcFuncList) -> VmrtRes<bool> {
    let mut replaced = false;
    for func in src.as_list() {
        let mut hit = false;
        for item in dst.as_mut() {
            if item.sign == func.sign {
                *item = func.clone();
                hit = true;
                replaced = true;
                break;
            }
        }
        if !hit {
            dst.push(func.clone()).map_ire(ContractUpgradeErr)?;
        }
    }
    Ok(replaced)
}

fn merge_user_funcs(dst: &mut ContractUserFuncList, src: &ContractUserFuncList) -> VmrtRes<bool> {
    let mut edit = false;
    for func in src.as_list() {
        let mut replaced = false;
        for item in dst.as_mut() {
            if item.sign == func.sign {
                *item = func.clone();
                replaced = true;
                edit = true;
                break;
            }
        }
        if !replaced {
            dst.push(func.clone()).map_ire(ContractUpgradeErr)?;
        }
    }
    Ok(edit)
}

impl ContractSto {
    pub fn calc_edition(&self) -> ContractEdition {
        ContractEdition {
            revision: self.metas.revision,
            raw_size: Uint4::from(field::Encode::size(self) as u32),
            hash: Hash::from(sys::calculate_hash(self.encode())),
        }
    }

    #[cfg(feature = "execute")]
    pub fn apply_edit(
        &mut self,
        edit: &ContractEdit,
        height: u64,
        cap: &SpaceCap,
        gas: &GasExtra,
        registry: &dyn base::ExecutionServices,
    ) -> VmrtRes<bool> {
        use ItrErrCode::*;

        let old_rev = self.metas.revision.uint();
        let Some(next_rev) = old_rev.checked_add(1) else {
            return itr_err_fmt!(ContractError, "contract revision overflow");
        };
        if edit.new_revision.uint() != next_rev {
            return itr_err_fmt!(
                ContractError,
                "contract revision mismatch: requested new_revision {} but next revision must be {}",
                edit.new_revision.uint(),
                next_rev
            );
        }

        // v0 inheritance is fixed at deployment; retain the edit fields on the wire.
        if edit.inherit_add.length() > 0 || edit.inherit_replace_at.length() > 0 {
            return itr_err_fmt!(InheritError, "contract inherit cannot be changed after deployment");
        }

        let edit_empty = edit.inherit_add.length() == 0
            && edit.inherit_replace_at.length() == 0
            && edit.library_add.length() == 0
            && edit.library_replace_at.length() == 0
            && edit.abstcalls.length() == 0
            && edit.userfuncs.length() == 0
            && edit.calcfuncs.length() == 0;
        if edit_empty {
            return itr_err_fmt!(ContractError, "contract edit is empty");
        }
        let mut did_change = false;

        if edit.calcfuncs.length() > 0 {
            if !calcfunc_enabled_at(height) {
                return itr_err_fmt!(ContractError, "calcfunc not enabled yet");
            }
            let mut seen = HashSet::new();
            for f in edit.calcfuncs.as_list() {
                let key = f.sign.into_array();
                if !seen.insert(key) {
                    return itr_err_fmt!(
                        ContractUpgradeErr,
                        "calcfunc sign already exists in edit"
                    );
                }
            }
            merge_calc_funcs(&mut self.calcfuncs, &edit.calcfuncs)?;
            did_change = true;
        }

        let inh_len = self.inherit.length();
        let lib_len = self.library.length();

        if edit.inherit_replace_at.length() > 0 {
            did_change = true;
            let mut idxs = HashSet::new();
            for r in edit.inherit_replace_at.as_list() {
                r.addr.check().map_ire(ContractAddrErr)?;
                let idx = r.idx_usize();
                if !idxs.insert(r.idx.uint()) {
                    return itr_err_fmt!(InheritError, "inherit replace index already exists");
                }
                if idx >= inh_len {
                    return itr_err_fmt!(InheritError, "inherit replace index overflow");
                }
                list_replace(self.inherit.as_mut(), idx, r.addr, InheritError)?;
            }
        }

        if edit.library_replace_at.length() > 0 {
            did_change = true;
            let mut idxs = HashSet::new();
            for r in edit.library_replace_at.as_list() {
                r.addr.check().map_ire(ContractAddrErr)?;
                let idx = r.idx_usize();
                if !idxs.insert(r.idx.uint()) {
                    return itr_err_fmt!(LibraryError, "library replace index already exists");
                }
                if idx >= lib_len {
                    return itr_err_fmt!(LibraryError, "library replace index overflow");
                }
                list_replace(self.library.as_mut(), idx, r.addr, LibraryError)?;
            }
        }

        if self.inherit.length() + edit.inherit_add.length() > cap.inherit {
            return itr_err_fmt!(InheritError, "inherit number overflow");
        }
        if self.library.length() + edit.library_add.length() > cap.library {
            return itr_err_fmt!(LibraryError, "library link number overflow");
        }

        if edit.inherit_add.length() > 0 {
            for a in edit.inherit_add.as_list() {
                a.check().map_ire(ContractAddrErr)?;
            }
            append_checked(
                self.inherit.as_mut(),
                edit.inherit_add.as_list(),
                InheritError,
            )?;
        }
        if edit.library_add.length() > 0 {
            for a in edit.library_add.as_list() {
                a.check().map_ire(ContractAddrErr)?;
            }
            append_checked(
                self.library.as_mut(),
                edit.library_add.as_list(),
                LibraryError,
            )?;
        }

        if edit.abstcalls.length() > 0 {
            did_change = true;
            let mut seen = HashSet::new();
            for a in edit.abstcalls.as_list() {
                if a.sign[0] == AbstCall::Construct as u8 {
                    return itr_err_fmt!(
                        ContractUpgradeErr,
                        "contract update cannot carry Construct abstcall"
                    );
                }
                if !seen.insert(a.sign[0]) {
                    return itr_err_fmt!(
                        ContractUpgradeErr,
                        "abstcall sign already exists in edit"
                    );
                }
            }
            for a in edit.abstcalls.as_list() {
                a.check(height)?;
                AbstCall::check(a.sign[0])?;
                verify_code_stuff(cap, gas, &a.code_stuff, height, registry)?;
            }
            merge_abst_funcs(&mut self.abstcalls, &edit.abstcalls)?;
        }

        if edit.userfuncs.length() > 0 {
            let mut seen = HashSet::new();
            for a in edit.userfuncs.as_list() {
                let key = a.sign.into_array();
                if !seen.insert(key) {
                    return itr_err_fmt!(
                        ContractUpgradeErr,
                        "userfunc sign already exists in edit"
                    );
                }
            }
            for a in edit.userfuncs.as_list() {
                a.check(height)?;
                verify_code_stuff(cap, gas, &a.code_stuff, height, registry)?;
            }
            if merge_user_funcs(&mut self.userfuncs, &edit.userfuncs)? {
                did_change = true;
            }
        }

        self.metas.revision = Uint2::from(next_rev);
        // Validate content only: the calcfunc activation gate applies to
        // newly supplied calcfuncs above, not to kernels the contract already
        // carries. Otherwise an unrelated edit would freeze a contract that
        // deployed its calcfuncs while the channel was enabled.
        self.check_content(height, cap, gas, registry)?;
        Ok(did_change)
    }

    pub fn have_abst_call(&self, ac: AbstCall) -> bool {
        self.abstcalls
            .as_list()
            .iter()
            .any(|a| ac as u8 == a.sign[0])
    }

    #[cfg(feature = "execute")]
    pub fn check(
        &self,
        height: u64,
        cap: &SpaceCap,
        gas: &GasExtra,
        registry: &dyn base::ExecutionServices,
    ) -> VmrtErr {
        // Deploy / explicit-upgrade entry: a state carrying calcfuncs cannot
        // be deployed while the channel is disabled.
        if self.calcfuncs.length() > 0 && !calcfunc_enabled_at(height) {
            return itr_err_fmt!(ContractError, "calcfunc not enabled yet");
        }
        self.check_content(height, cap, gas, registry)
    }

    /// Validation of contract content, excluding the calcfunc activation
    /// gate. Shared by deploy (`check`) and upgrade (`apply_edit`) so a state
    /// loaded or upgraded outside the deploy path keeps the same invariants
    /// without being frozen by the channel window.
    #[cfg(feature = "execute")]
    fn check_content(
        &self,
        height: u64,
        cap: &SpaceCap,
        gas: &GasExtra,
        registry: &dyn base::ExecutionServices,
    ) -> VmrtErr {
        use ItrErrCode::*;

        self.metas.check(height)?;
        if self.morextend.uint() != 0 {
            return itr_err_fmt!(ContractError, "morextend reserved, must be zero");
        }
        if field::Encode::size(self) > cap.contract_size {
            return itr_err_fmt!(
                ContractError,
                "contract size overflow, max {}",
                cap.contract_size
            );
        }
        if self.calcfuncs.length() > 0 {
            // Cheap bounds before any per-kernel CRC/decode/verify work. A
            // `CALC_CALL` index is a single byte, so more than 256 kernels can
            // never be addressed; a container larger than `function_size`
            // cannot unroll to a valid kernel either.
            if self.calcfuncs.length() > 256 {
                return itr_err_fmt!(ContractError, "calcfunc number overflow, max 256");
            }
            for f in self.calcfuncs.as_list() {
                if f.code_stuff.data.len() > cap.function_size {
                    return itr_err_fmt!(
                        ContractError,
                        "calcfunc size overflow, max {}",
                        cap.function_size
                    );
                }
            }
            // mark is the ABI form (1 = bytes->bytes) and every payload must
            // decode + verify as a calc container
            let mut seen = HashSet::new();
            for f in self.calcfuncs.as_list() {
                let key = f.sign.into_array();
                if !seen.insert(key) {
                    return itr_err_fmt!(ContractError, "calcfunc sign already exists");
                }
            }
            for f in self.calcfuncs.as_list() {
                let (c, _used) =
                    crate::calc::container::CalcContainer::decode(&f.code_stuff.data).map_err(
                        |e| ItrErr::new(ContractError, &format!("calcfunc container: {}", e.1)),
                    )?;
                validate_calc_func(f, &c)?;
                // A kernel whose declared IO exceeds the HVM call-value cap can
                // never be invoked or return its result; reject it at deploy /
                // upgrade time instead of accepting a permanently unusable fn.
                if c.input_max as usize > cap.value_size || c.output_max as usize > cap.value_size {
                    return itr_err_fmt!(ContractError, "calcfunc io exceeds HVM value_size");
                }
                crate::calc::verify::verify_container(&c).map_err(|e| {
                    ItrErr::new(ContractError, &format!("calcfunc verify: {}", e.1))
                })?;
            }
        }
        if self.inherit.length() > cap.inherit {
            return itr_err_fmt!(InheritError, "inherit number overflow");
        }
        if self.library.length() > cap.library {
            return itr_err_fmt!(LibraryError, "library link number overflow");
        }

        let mut inhset = HashSet::new();
        for a in self.inherit.as_list() {
            a.check().map_ire(ContractAddrErr)?;
            if !inhset.insert(*a) {
                return itr_err_fmt!(InheritError, "inherit already exists");
            }
        }
        let mut libset = HashSet::new();
        for a in self.library.as_list() {
            a.check().map_ire(ContractAddrErr)?;
            if !libset.insert(*a) {
                return itr_err_fmt!(LibraryError, "library already exists");
            }
        }

        let mut abst_seen = HashSet::new();
        for a in self.abstcalls.as_list() {
            if !abst_seen.insert(a.sign[0]) {
                return itr_err_fmt!(ContractError, "abstcall sign already exists");
            }
        }
        for a in self.abstcalls.as_list() {
            a.check(height)?;
            AbstCall::check(a.sign[0])?;
            verify_code_stuff(cap, gas, &a.code_stuff, height, registry)?;
        }

        let mut user_seen = HashSet::new();
        for a in self.userfuncs.as_list() {
            let key = a.sign.into_array();
            if !user_seen.insert(key) {
                return itr_err_fmt!(ContractError, "userfunc sign already exists");
            }
        }
        for a in self.userfuncs.as_list() {
            a.check(height)?;
            verify_code_stuff(cap, gas, &a.code_stuff, height, registry)?;
        }

        Ok(())
    }

    #[cfg(feature = "execute")]
    pub fn into_obj(mut self, cap: &SpaceCap) -> VmrtRes<ContractObj> {
        let edition = self.calc_edition();
        let mut abstfns = HashMap::with_capacity(self.abstcalls.length());
        for a in self.abstcalls.as_mut() {
            let code_pkg = CodePkg::try_from(std::mem::take(&mut a.code_stuff))?;
            let code = FnObj::create(a.fncnf[0], code_pkg, None)?;
            let cty = AbstCall::try_from_u8(a.sign[0])?;
            abstfns.insert(cty, Arc::new(code));
        }

        let mut userfns = HashMap::with_capacity(self.userfuncs.length());
        for a in self.userfuncs.as_mut() {
            let code_pkg = CodePkg::try_from(std::mem::take(&mut a.code_stuff))?;
            let code = FnObj::create(a.fncnf[0], code_pkg, Some(a.pmdf.clone()))?;
            userfns.insert(a.sign.into_array(), Arc::new(code));
        }

        let mut calcfns = Vec::with_capacity(self.calcfuncs.length());
        for f in self.calcfuncs.as_mut() {
            // Mirror the deploy path checks in `check_content`: a state loaded
            // outside the deploy path must not bypass the per-kernel byte cap,
            // the ABI / reserved-conf / fncnf agreement, or the HVM IO cap.
            if f.code_stuff.data.len() > cap.function_size {
                return itr_err_fmt!(
                    ContractError,
                    "calcfunc size overflow, max {}",
                    cap.function_size
                );
            }
            let (c, _used) = crate::calc::container::CalcContainer::decode(&f.code_stuff.data)?;
            validate_calc_func(f, &c)?;
            if c.input_max as usize > cap.value_size || c.output_max as usize > cap.value_size {
                return itr_err_fmt!(ContractError, "calcfunc io exceeds HVM value_size");
            }
            calcfns.push(Arc::new(crate::calc::CalcFnObj::build(c)?));
        }

        Ok(ContractObj {
            sto: self,
            abstfns,
            userfns,
            calcfns,
            edition,
        })
    }
}

#[cfg(all(test, feature = "execute"))]
mod calc_io_load_tests {
    use super::*;
    use crate::machine::test_ctx::{StubServices, STUB_VM_PARAMS};
    use field::{BytesW2, Fixed1, Fixed4, Uint1};

    /// `addi a0, x0, 0 ; jalr x0, x1, 0` - a verifier-clean kernel body.
    const KERNEL_CODE: [u8; 8] = [0x13, 0x05, 0, 0, 0x67, 0x80, 0, 0];

    fn calcfunc(input_max: u16, output_max: u16, mark: u8, fncnf: u8) -> ContractCalcFunc {
        let c = crate::calc::pack_container(
            crate::calc::container::CALC_ISA_RV64IMC,
            64,
            0,
            input_max,
            output_max,
            KERNEL_CODE.to_vec(),
            vec![],
            vec![],
        )
        .expect("test container must pack");
        ContractCalcFunc {
            sign: Fixed4::from([1, 2, 3, 4]),
            mark: Fixed1::from([mark]),
            fncnf: Fixed1::from([fncnf]),
            code_stuff: CodeStuff {
                conf: Uint1::from(0),
                data: BytesW2::from(c.encode().unwrap()).unwrap(),
            },
        }
    }

    fn check_at_100(sto: &ContractSto) -> VmrtErr {
        let cap = SpaceCap::new(0);
        let gas = GasExtra::new(0, &STUB_VM_PARAMS);
        let svc = StubServices::new(STUB_VM_PARAMS);
        sto.check(100, &cap, &gas, &svc)
    }

    #[test]
    fn output_max_exceeds_hvm_value_size_is_rejected() {
        let mut sto = ContractSto::default();
        // 4096 fits the calc container wire limit (CALC_IO_MAX) but exceeds
        // the HVM value cap (SpaceCap::value_size == 1280).
        sto.calcfuncs.push(calcfunc(16, 4096, 1, 0b11)).unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(
            e.1.contains("io exceeds HVM value_size"),
            "unexpected error: {:?}",
            e
        );
    }

    #[test]
    fn io_within_hvm_value_size_is_accepted() {
        let mut sto = ContractSto::default();
        sto.calcfuncs.push(calcfunc(16, 16, 1, 0b11)).unwrap();
        check_at_100(&sto).expect("io within value_size must pass check");
    }

    #[test]
    fn load_path_rejects_bad_mark() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        sto.calcfuncs.push(calcfunc(16, 16, 0, 0b11)).unwrap();
        assert!(check_at_100(&sto).is_err(), "check must reject mark 0");
        // Before the loader-side re-check this was accepted by `into_obj`.
        let e = sto
            .into_obj(&cap)
            .err()
            .expect("into_obj must reject mark 0");
        assert!(e.1.contains("mark (abi) must be 1"), "unexpected: {:?}", e);
    }

    #[test]
    fn load_path_rejects_fncnf_mismatch() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        // Container fncnf is 0b11 (pack_container), the contract field lies.
        sto.calcfuncs.push(calcfunc(16, 16, 1, 0b10)).unwrap();
        let e = sto
            .into_obj(&cap)
            .err()
            .expect("into_obj must reject fncnf mismatch");
        assert!(
            e.1.contains("fncnf mismatch with container"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn load_path_rejects_output_max_over_hvm_value_size() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        // Valid in the calc wire format but unusable under the HVM value cap.
        sto.calcfuncs.push(calcfunc(16, 4096, 1, 0b11)).unwrap();
        let e = sto
            .into_obj(&cap)
            .err()
            .expect("into_obj must reject io overflow");
        assert!(
            e.1.contains("io exceeds HVM value_size"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn reserved_conf_must_be_zero_for_check_and_load() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        let mut f = calcfunc(16, 16, 1, 0b11);
        f.code_stuff.conf = Uint1::from(1);
        sto.calcfuncs.push(f).unwrap();

        let e = check_at_100(&sto).unwrap_err();
        assert!(
            e.1.contains("code conf reserved, must be zero"),
            "unexpected: {:?}",
            e
        );

        let e = sto
            .into_obj(&cap)
            .err()
            .expect("into_obj must reject conf=1");
        assert!(
            e.1.contains("code conf reserved, must be zero"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn calcfunc_count_over_u8_idx_limit_is_rejected_before_decode() {
        let mut sto = ContractSto::default();
        for i in 0..257u32 {
            let mut f = calcfunc(16, 16, 1, 0b11);
            f.sign = Fixed4::from(i.to_le_bytes());
            // Corrupt payload: this would fail decode if the count check ran
            // after the decode/verify loop.
            f.code_stuff.data = BytesW2::from(Vec::new()).unwrap();
            sto.calcfuncs.push(f).unwrap();
        }
        let e = check_at_100(&sto).unwrap_err();
        assert!(
            e.1.contains("calcfunc number overflow, max 256"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn calcfunc_payload_over_function_size_is_rejected_before_decode() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        let mut f = calcfunc(16, 16, 1, 0b11);
        // Junk larger than the per-kernel cap: rejected before decode/verify.
        f.code_stuff.data = BytesW2::from(vec![0u8; cap.function_size + 1]).unwrap();
        sto.calcfuncs.push(f).unwrap();

        let gas = GasExtra::new(0, &STUB_VM_PARAMS);
        let svc = StubServices::new(STUB_VM_PARAMS);
        let e = sto.check(100, &cap, &gas, &svc).unwrap_err();
        assert!(
            e.1.contains("calcfunc size overflow"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn upgrade_with_existing_calcfuncs_survives_the_gate_window() {
        let cap = SpaceCap::new(0);
        let gas = GasExtra::new(0, &STUB_VM_PARAMS);
        let svc = StubServices::new(STUB_VM_PARAMS);
        let height = CALCFUNC_DISABLE_FROM_HEIGHT;

        let mut sto = ContractSto::default();
        sto.calcfuncs.push(calcfunc(16, 16, 1, 0b11)).unwrap();

        // Deploy-style validation remains gated inside the freeze window.
        let e = sto
            .check(height, &cap, &gas, &svc)
            .expect_err("deploy-style check must stay gated");
        assert!(e.1.contains("calcfunc not enabled yet"), "unexpected: {:?}", e);

        // An unrelated library edit must not freeze an existing contract.
        let addr = ContractAddress::calculate(
            &field::Address::from([0u8; field::Address::SIZE]),
            &Uint4::from(1),
        );
        let edit = ContractEdit {
            new_revision: Uint2::from(1),
            inherit_add: Default::default(),
            inherit_replace_at: Default::default(),
            library_add: ContractAddrListW1::from(vec![addr]).unwrap(),
            library_replace_at: Default::default(),
            abstcalls: Default::default(),
            userfuncs: Default::default(),
            calcfuncs: Default::default(),
        };
        let _changed = sto
            .apply_edit(&edit, height, &cap, &gas, &svc)
            .expect("library-only upgrade must pass during the freeze window");
        assert_eq!(sto.library.length(), 1);
        assert_eq!(sto.metas.revision.uint(), 1);
    }
}

#[cfg(test)]
mod calc_gate_window_tests {
    use super::{
        calcfunc_enabled_at, CALCFUNC_DISABLE_FROM_HEIGHT, CALCFUNC_DISABLE_UNTIL_HEIGHT,
    };

    #[test]
    fn gate_is_disabled_only_in_the_configured_window() {
        assert!(calcfunc_enabled_at(CALCFUNC_DISABLE_FROM_HEIGHT - 1));
        assert!(!calcfunc_enabled_at(CALCFUNC_DISABLE_FROM_HEIGHT));
        assert!(!calcfunc_enabled_at(CALCFUNC_DISABLE_UNTIL_HEIGHT - 1));
        assert!(calcfunc_enabled_at(CALCFUNC_DISABLE_UNTIL_HEIGHT));
    }
}
