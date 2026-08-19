use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use field::{
    Decode, Encode, Fixed1, Fixed2, Fixed3, Fixed4, Hash, ListW1, ListW2, Reader, Uint1, Uint2,
    Uint4, Uint8,
};
use sys::Ret;

use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::ContractAddress;

mod schema;

pub type ContractAddrListW1 = ListW1<ContractAddress>;
pub type ContractAbstCallList = ListW1<ContractAbstCall>;
pub type ContractUserFuncList = ListW2<ContractUserFunc>;
pub type ContractCalcFuncList = ListW2<ContractCalcFunc>;
pub type ContractAddrReplaceAtList = ListW1<ContractAddrReplaceAt>;

macro_rules! contract_codec_struct {
    ($name:ident { $($field:ident : $ty:ty),+ $(,)? }) => {
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub struct $name {
            $(pub $field: $ty),+
        }

        impl Encode for $name {
            fn size(&self) -> usize {
                0 $(+ field::Encode::size(&self.$field))+
            }

            fn encode_to(&self, out: &mut Vec<u8>) {
                $(self.$field.encode_to(out);)+
            }
        }

        impl Decode for $name {
            fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
                let mut r = Reader::new(buf);
                $(let $field: $ty = r.read()?;)+
                Ok((Self { $($field),+ }, r.used()))
            }
        }

        // Wire schema (struct + field shapes) comes from the shared macro; the
        // field names on the wire match the struct fields 1:1.
        field::wire_struct_schema!($name { $($field: $ty),+ });
    };
}

/// Defines the contract wire structs in one place: struct + codec + JSON +
/// schema registration (`struct_schemas()`), so adding a struct only touches
/// the single invocation below.
macro_rules! contract_structs {
    ($( $name:ident { $($field:ident : $ty:ty),+ $(,)? } ),+ $(,)?) => {
        $(contract_codec_struct!($name { $($field: $ty),+ });)+
        $(base::impl_fields_to_json!($name { $($field),+ });)+
        pub fn struct_schemas() -> Vec<base::StructSchema> {
            vec![$(<$name as base::StructSchemaProvider>::STRUCT_SCHEMA),+]
        }
    };
}

contract_structs! {
    ContractMeta {
        vrsn: Fixed1,
        revision: Uint2,
        mark: Fixed3,
        mext: Fixed4,
    },
    ContractAbstCall {
        sign: Fixed1,
        mark: Fixed2,
        fncnf: Fixed1,
        code_stuff: CodeStuff,
    },
    ContractUserFunc {
        sign: Fixed4,
        mark: Fixed3,
        fncnf: Fixed1,
        pmdf: FuncArgvTypes,
        code_stuff: CodeStuff,
    },
    ContractCalcFunc {
        sign: Fixed4,
        mark: Fixed1,
        fncnf: Fixed1,
        code_stuff: CodeStuff,
    },
    ContractAddrReplaceAt {
        idx: Uint1,
        addr: ContractAddress,
    },
    ContractEdit {
        new_revision: Uint2,
        inherit_add: ContractAddrListW1,
        inherit_replace_at: ContractAddrReplaceAtList,
        library_add: ContractAddrListW1,
        library_replace_at: ContractAddrReplaceAtList,
        abstcalls: ContractAbstCallList,
        userfuncs: ContractUserFuncList,
        calcfuncs: ContractCalcFuncList,
    },
    ContractSto {
        metas: ContractMeta,
        inherit: ContractAddrListW1,
        library: ContractAddrListW1,
        abstcalls: ContractAbstCallList,
        userfuncs: ContractUserFuncList,
        calcfuncs: ContractCalcFuncList,
        morextend: Uint8,
    },
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

impl ContractCalcFunc {
    #[allow(dead_code)]
    fn check(&self, _height: u64) -> VmrtErr {
        if !self.mark.is_zero() || !self.fncnf.is_zero() {
            return itr_err_fmt!(ContractError, "contract ContractCalcFunc format invalid");
        }
        Ok(())
    }
}

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
        if edit.calcfuncs.length() > 0 {
            return itr_err_fmt!(ContractError, "calcfunc not enabled yet");
        }

        let mut did_change = false;
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
        self.check(height, cap, gas, registry)?;
        Ok(did_change)
    }

    pub fn have_abst_call(&self, ac: AbstCall) -> bool {
        self.abstcalls
            .as_list()
            .iter()
            .any(|a| ac as u8 == a.sign[0])
    }

    pub fn check(
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
        if self.calcfuncs.length() != 0 {
            return itr_err_fmt!(ContractError, "calcfunc not enabled yet");
        }
        if field::Encode::size(self) > cap.contract_size {
            return itr_err_fmt!(
                ContractError,
                "contract size overflow, max {}",
                cap.contract_size
            );
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

    pub fn into_obj(mut self) -> VmrtRes<ContractObj> {
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

        Ok(ContractObj {
            sto: self,
            abstfns,
            userfns,
            edition,
        })
    }
}
