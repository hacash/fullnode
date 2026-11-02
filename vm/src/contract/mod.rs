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
/// window, so deployment / upgrade keep rejecting `CodeType::RV64IM` user
/// functions. The channel
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

/// Permanently retired wire record, kept only for codec stability — it will
/// never be activated. Calc kernels are `ContractUserFunc` values whose
/// `code_stuff` uses `CodeType::RV64IM`; `ContractSto.calcfuncs` must stay
/// empty forever.
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

#[cfg(feature = "execute")]
fn code_stuff_is_rv64im(code_stuff: &CodeStuff) -> bool {
    code_stuff
        .parse_conf()
        .is_ok_and(|c| c.code_type() == CodeType::RV64IM)
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
        CodeType::RV64IM => {
            return itr_err_fmt!(
                CodeTypeError,
                "calc code type is only valid on contract user functions"
            );
        }
    };
    if bytecodes.len() > cap.function_size {
        return itr_err_code!(CodeTooLong);
    }
    verify_bytecodes_for_cap(&bytecodes, cap.value_size, registry)
}

#[cfg(feature = "execute")]
fn calc_deploy_config(cap: &SpaceCap) -> crate::calc::CalcConfig {
    let mut cfg = crate::calc::CalcConfig::v1();
    cfg.mem_kib_max = cap
        .calc_mem_kib_max
        .min(crate::calc::container::CALC_MEM_KIB_ENGINE_MAX);
    cfg
}

#[cfg(feature = "execute")]
fn reject_calc_image(len: usize, cap: &SpaceCap) -> VmrtErr {
    // CodeStuff stores every function body with a u16 length. calc_image_max
    // can only tighten that. A larger program is split across sibling calc
    // functions later, not by widening this prefix.
    let max = cap.calc_image_max.min(u16::MAX as usize);
    if len > max {
        return itr_err_fmt!(ContractError, "calcfunc size overflow, max {}", max);
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn reject_calc_hvm_io(c: &crate::calc::container::CalcContainer, cap: &SpaceCap) -> VmrtErr {
    if c.input_max as usize > cap.call_data_size {
        return itr_err_fmt!(ContractError, "calcfunc input_max exceeds call_data_size");
    }
    if c.output_max as usize > cap.value_size {
        return itr_err_fmt!(ContractError, "calcfunc output_max exceeds HVM value_size");
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn verify_user_func_code(
    cap: &SpaceCap,
    gas: &GasExtra,
    func: &ContractUserFunc,
    height: u64,
    registry: &dyn base::ExecutionServices,
) -> VmrtErr {
    let code_pkg = CodePkg::try_from(&func.code_stuff)?;
    match code_pkg.code_type()? {
        CodeType::Bytecode | CodeType::IRNode => {
            verify_code_stuff(cap, gas, &func.code_stuff, height, registry)
        }
        CodeType::RV64IM => {
            if func.fncnf[0] != 0 {
                return itr_err_fmt!(ContractError, "calc function cannot be external");
            }
            reject_calc_image(code_pkg.data.len(), cap)?;
            let (c, _used) = crate::calc::container::CalcContainer::decode(&code_pkg.data)
                .map_err(|e| ItrErr::new(ContractError, &format!("calcfunc container: {}", e.1)))?;
            reject_calc_hvm_io(&c, cap)?;
            check_rv64im_pmdf(&func.pmdf)?;
            crate::calc::verify::verify_container_with(&c, &calc_deploy_config(cap))
                .map_err(|e| ItrErr::new(ContractError, &format!("calcfunc verify: {}", e.1)))?;
            Ok(())
        }
    }
}

#[cfg(feature = "execute")]
fn check_rv64im_pmdf(pmdf: &FuncArgvTypes) -> VmrtErr {
    let params = pmdf.param_types().map_ire(ContractError)?;
    match params.as_slice() {
        [] | [crate::ValueTy::Bytes] => {}
        _ => {
            return itr_err_fmt!(
                ContractError,
                "calc pmdf params must be empty or one bytes"
            );
        }
    }
    match pmdf.output_type().map_ire(ContractError)? {
        None | Some(crate::ValueTy::Bytes) => Ok(()),
        _ => itr_err_fmt!(ContractError, "calc pmdf output must be none or bytes"),
    }
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
            return itr_err_fmt!(ContractError, "calcfuncs reserved, must be empty");
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
            if edit
                .userfuncs
                .as_list()
                .iter()
                .any(|f| code_stuff_is_rv64im(&f.code_stuff))
                && !calcfunc_enabled_at(height)
            {
                return itr_err_fmt!(ContractError, "calcfunc not enabled yet");
            }
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
                verify_user_func_code(cap, gas, a, height, registry)?;
            }
            if merge_user_funcs(&mut self.userfuncs, &edit.userfuncs)? {
                did_change = true;
            }
        }

        self.metas.revision = Uint2::from(next_rev);
        // The activation gate applies only to calc user functions supplied in
        // this edit. An unrelated edit must not freeze a contract that already
        // carries `CodeType::RV64IM` functions.
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
        if self
            .userfuncs
            .as_list()
            .iter()
            .any(|f| code_stuff_is_rv64im(&f.code_stuff))
            && !calcfunc_enabled_at(height)
        {
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
            return itr_err_fmt!(ContractError, "calcfuncs reserved, must be empty");
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
            verify_user_func_code(cap, gas, a, height, registry)?;
        }

        Ok(())
    }

    #[cfg(feature = "execute")]
    pub fn into_obj(self, cap: &SpaceCap) -> VmrtRes<ContractObj> {
        self.into_obj_with(cap, crate::calc::CalcHostConfig::new())
    }

    #[cfg(feature = "execute")]
    pub fn into_obj_with(
        mut self,
        cap: &SpaceCap,
        host: crate::calc::CalcHostConfig,
    ) -> VmrtRes<ContractObj> {
        if self.calcfuncs.length() > 0 {
            return itr_err_fmt!(
                ItrErrCode::ContractError,
                "calcfuncs reserved, must be empty"
            );
        }
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
            let code = if code_pkg.code_type()? == CodeType::RV64IM {
                if a.fncnf[0] != 0 {
                    return itr_err_fmt!(ContractError, "calc function cannot be external");
                }
                reject_calc_image(code_pkg.data.len(), cap)?;
                let (c, _) = crate::calc::container::CalcContainer::decode(&code_pkg.data)?;
                reject_calc_hvm_io(&c, cap)?;
                check_rv64im_pmdf(&a.pmdf)?;
                let cfg = calc_deploy_config(cap).apply_host(host);
                let built = crate::calc::CalcFnObj::build_with_config(c, cfg)?;
                FnObj::create_with_rv64im(
                    a.fncnf[0],
                    code_pkg,
                    Some(a.pmdf.clone()),
                    Arc::new(built),
                )?
            } else {
                FnObj::create(a.fncnf[0], code_pkg, Some(a.pmdf.clone()))?
            };
            userfns.insert(a.sign.into_array(), Arc::new(code));
        }

        Ok(ContractObj {
            sto: self,
            abstfns,
            userfns,
            edition,
        })
    }
}

#[cfg(all(test, feature = "execute"))]
mod calc_io_load_tests {
    use super::*;
    use crate::machine::test_ctx::{StubServices, STUB_VM_PARAMS};
    use field::{BytesW2, Fixed1, Fixed4, Uint1};

    /// HALT ecall (exit 0, empty output) - a verifier-clean kernel body.
    const KERNEL_CODE: [u8; 16] = [
        0x13, 0x05, 0, 0, 0x93, 0x05, 0, 0, 0x93, 0x08, 0, 0, 0x73, 0, 0, 0,
    ];

    fn calc_user(input_max: u16, output_max: u16) -> ContractUserFunc {
        let c = crate::calc::pack_container(
            64,
            crate::calc::CALC_CODE_BASE as u32,
            input_max,
            output_max,
            KERNEL_CODE.to_vec(),
            vec![],
            vec![],
        )
        .expect("test container must pack");
        let mut func = ContractUserFunc::default();
        func.sign = Fixed4::from([1, 2, 3, 4]);
        func.code_stuff = CodeStuff {
            conf: Uint1::from(CodeType::RV64IM as u8),
            data: BytesW2::from(c.encode().unwrap()).unwrap(),
        };
        func
    }

    fn user_from_bytes(sign: [u8; 4], bytes: Vec<u8>) -> ContractUserFunc {
        let mut func = ContractUserFunc::default();
        func.sign = Fixed4::from(sign);
        func.code_stuff = CodeStuff {
            conf: Uint1::from(CodeType::RV64IM as u8),
            data: BytesW2::from(bytes).unwrap(),
        };
        func
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
        // 4096 fits the container field but exceeds the HVM value cap
        // (SpaceCap::value_size == 1280).
        sto.userfuncs.push(calc_user(16, 4096)).unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(
            e.1.contains("output_max exceeds HVM value_size"),
            "unexpected error: {:?}",
            e
        );
    }

    #[test]
    fn input_max_at_calc_io_max_is_accepted() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        sto.userfuncs.push(calc_user(5120, 16)).unwrap();
        check_at_100(&sto).expect("input_max 5120 equals L1 call_data_size");
        sto.into_obj(&cap).expect("load uses the same call_data_size");

        let mut over = ContractSto::default();
        over.userfuncs.push(calc_user(6000, 16)).unwrap();
        let e = check_at_100(&over).unwrap_err();
        assert!(e.1.contains("call_data_size"), "{e:?}");
        let e = over.into_obj(&cap).err().expect("load rejects the same input_max");
        assert!(e.1.contains("call_data_size"), "{e:?}");
    }

    #[test]
    fn io_within_hvm_value_size_is_accepted() {
        let mut sto = ContractSto::default();
        sto.userfuncs.push(calc_user(16, 16)).unwrap();
        check_at_100(&sto).expect("io within value_size must pass check");
        assert_eq!(sto.calcfuncs.length(), 0);
        assert_eq!(sto.userfuncs[0].code_stuff.conf.uint(), CodeType::RV64IM as u8);
    }

    #[test]
    fn deploy_and_load_share_the_space_cap() {
        let bytes = {
            let mut raw = calc_user(16, 16).code_stuff.data.to_vec();
            raw[5] = crate::calc::CALC_PROFILE_RV64IM_B;
            raw[8..12].copy_from_slice(&65u32.to_be_bytes());
            raw
        };
        let sto = || {
            let mut sto = ContractSto::default();
            sto.userfuncs
                .push(user_from_bytes([8, 8, 8, 8], bytes.clone()))
                .unwrap();
            sto
        };
        let cap = SpaceCap::new(0);
        let gas = GasExtra::new(0, &STUB_VM_PARAMS);
        let svc = StubServices::new(STUB_VM_PARAMS);
        let e = sto().check(100, &cap, &gas, &svc).unwrap_err();
        assert!(e.1.contains("mem_kib"), "{e:?}");
        let e = sto().into_obj(&cap).err().expect("load rejects the same mem cap");
        assert!(e.1.contains("mem_kib"), "{e:?}");

        let mut wide = SpaceCap::new(0);
        wide.calc_mem_kib_max = 128;
        sto().check(100, &wide, &gas, &svc)
            .expect("raised mem cap admits the kernel");
        sto().into_obj(&wide)
            .expect("load uses the same mem cap");
    }

    #[test]
    fn rv64im_pmdf_rejects_non_bytes() {
        let mut sto = ContractSto::default();
        let mut func = calc_user(16, 16);
        func.pmdf = FuncArgvTypes::from_types(Some(crate::ValueTy::U64), vec![crate::ValueTy::U64])
            .unwrap();
        sto.userfuncs.push(func).unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(e.1.contains("calc pmdf"), "{e:?}");
    }

    #[test]
    fn mem_kib_above_v1_cap_is_rejected_at_deploy() {
        let c = crate::calc::pack_container(
            64,
            crate::calc::CALC_CODE_BASE as u32,
            16,
            16,
            KERNEL_CODE.to_vec(),
            vec![],
            vec![],
        )
        .expect("64 KiB container packs");
        let mut bytes = c.encode().unwrap();
        bytes[8..12].copy_from_slice(&65u32.to_be_bytes());
        let (decoded, _) = crate::calc::container::CalcContainer::decode(&bytes)
            .expect("wire decode accepts mem_kib=65");
        assert_eq!(decoded.mem_kib, 65);
        let mut sto = ContractSto::default();
        sto.userfuncs
            .push(user_from_bytes([9, 9, 9, 9], bytes))
            .unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(e.1.contains("calcfunc verify"), "{e:?}");
        assert!(e.1.contains("mem_kib"), "{e:?}");
    }

    #[test]
    fn load_path_rejects_external_calc() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        let mut func = calc_user(16, 16);
        func.fncnf = Fixed1::from([FnConf::External as u8]);
        sto.userfuncs.push(func).unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(e.1.contains("cannot be external"), "unexpected: {:?}", e);
        let e = sto
            .into_obj(&cap)
            .err()
            .expect("into_obj must reject external calc");
        assert!(e.1.contains("cannot be external"), "unexpected: {:?}", e);
    }


    #[test]
    fn load_path_rejects_output_max_over_hvm_value_size() {
        let cap = SpaceCap::new(0);
        let mut sto = ContractSto::default();
        // Valid in the calc wire format but unusable under the HVM value cap.
        sto.userfuncs.push(calc_user(16, 4096)).unwrap();
        let e = sto
            .into_obj(&cap)
            .err()
            .expect("into_obj must reject io overflow");
        assert!(
            e.1.contains("output_max exceeds HVM value_size"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn calcfuncs_list_is_reserved() {
        let mut sto = ContractSto::default();
        sto.calcfuncs.push(ContractCalcFunc::default()).unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(
            e.1.contains("calcfuncs reserved, must be empty"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn into_obj_indexes_calc_userfns_and_rejects_sign_collision() {
        let cap = SpaceCap::new(0);
        let f = calc_user(16, 16);
        let sign = f.sign.into_array();
        let mut sto = ContractSto::default();
        sto.userfuncs.push(f).unwrap();
        let obj = sto.into_obj(&cap).expect("into_obj indexes calc userfns");
        let got = obj.userfns.get(&sign).expect("calc fn lives in userfns");
        assert!(got.is_rv64im());
        assert!(got.rv64im_fn().is_some());
        assert_eq!(obj.sto.calcfuncs.length(), 0);

        let f = calc_user(16, 16);
        let mut user = ContractUserFunc::default();
        user.sign = f.sign.clone();
        let mut sto = ContractSto::default();
        sto.userfuncs.push(f).unwrap();
        sto.userfuncs.push(user).unwrap();
        let e = check_at_100(&sto).unwrap_err();
        assert!(
            e.1.contains("userfunc sign already exists"),
            "unexpected: {:?}",
            e
        );
    }

    #[test]
    fn calcfunc_payload_over_function_size_is_rejected_before_decode() {
        let mut cap = SpaceCap::new(0);
        cap.calc_image_max = 32;
        let mut sto = ContractSto::default();
        let mut f = calc_user(16, 16);
        // Junk larger than the calc image cap: rejected before decode/verify.
        // HVM `function_size` does not apply to this code type.
        f.code_stuff.data = BytesW2::from(vec![0u8; cap.calc_image_max + 1]).unwrap();
        sto.userfuncs.push(f).unwrap();

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
        sto.userfuncs.push(calc_user(16, 16)).unwrap();

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

    fn calc_edit(rev: u16, func: ContractUserFunc) -> ContractEdit {
        ContractEdit {
            new_revision: Uint2::from(rev),
            inherit_add: Default::default(),
            inherit_replace_at: Default::default(),
            library_add: Default::default(),
            library_replace_at: Default::default(),
            abstcalls: Default::default(),
            userfuncs: ContractUserFuncList::from(vec![func]).unwrap(),
            calcfuncs: Default::default(),
        }
    }

    #[test]
    fn apply_edit_calcfunc_append_is_not_a_change() {
        let cap = SpaceCap::new(0);
        let gas = GasExtra::new(0, &STUB_VM_PARAMS);
        let svc = StubServices::new(STUB_VM_PARAMS);
        let height = 100u64;
        let mut sto = ContractSto::default();

        let added = calc_user(16, 16);
        let changed = sto
            .apply_edit(&calc_edit(1, added.clone()), height, &cap, &gas, &svc)
            .expect("appending a new calcfunc");
        assert!(!changed, "fresh sign selects Append");
        assert_eq!(sto.userfuncs.length(), 1);
        assert_eq!(sto.calcfuncs.length(), 0);
        assert_eq!(sto.userfuncs[0].code_stuff.conf.uint(), CodeType::RV64IM as u8);

        let mut replaced = calc_user(32, 16);
        replaced.sign = added.sign;
        let changed = sto
            .apply_edit(&calc_edit(2, replaced), height, &cap, &gas, &svc)
            .expect("replacing an existing calcfunc");
        assert!(changed, "same sign selects Change");
        assert_eq!(sto.userfuncs.length(), 1);
        assert_eq!(sto.calcfuncs.length(), 0);
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
