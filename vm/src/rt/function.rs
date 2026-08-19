pub const FN_SIGN_WIDTH: usize = 4;
pub type FnSign = [u8; FN_SIGN_WIDTH];

pub fn calc_func_sign(name: &str) -> FnSign {
    field::Hash::from(sys::calculate_hash(name)).into_array()[..FN_SIGN_WIDTH]
        .try_into()
        .unwrap()
}

pub fn checked_func_sign(s: &[u8]) -> VmrtRes<FnSign> {
    if s.len() != FN_SIGN_WIDTH {
        return itr_err!(CastParamFail, "fn signature size invalid");
    }
    Ok(s.try_into().unwrap())
}

#[repr(u8)]
#[derive(Debug, Copy, Clone)]
pub enum FnConf {
    External = 0b10000000,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct FnObj {
    pub confs: u8,
    pub agvty: Option<FuncArgvTypes>,
    pub ctype: CodeType,
    pub codes: ByteView,
    compiled: Arc<std::sync::OnceLock<ByteView>>,
}

impl FnObj {
    pub fn check_conf(&self, cnf: FnConf) -> bool {
        let cnfset = cnf as u8;
        self.confs & cnfset == cnfset
    }

    pub fn create(fncnf: u8, pkg: CodePkg, agvty: Option<FuncArgvTypes>) -> VmrtRes<Self> {
        let ctype = pkg.code_type()?;
        Ok(Self {
            confs: fncnf,
            agvty,
            ctype,
            codes: ByteView::from_vec(pkg.data),
            compiled: Arc::new(std::sync::OnceLock::new()),
        })
    }

    pub fn plain(
        ctype: CodeType,
        codes: impl Into<ByteView>,
        confs: u8,
        agvty: Option<FuncArgvTypes>,
    ) -> Self {
        Self {
            confs,
            agvty,
            ctype,
            codes: codes.into(),
            compiled: Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub fn exec_bytecodes(&self, _height: u64, gas_extra: &GasExtra) -> VmrtRes<ByteView> {
        use CodeType::*;
        Ok(match self.ctype {
            Bytecode => self.codes.clone(),
            IRNode => {
                if let Some(cached) = self.compiled.get() {
                    return Ok(cached.clone());
                }
                let compiled = ByteView::from_vec(crate::ir::runtime_irs_to_exec_bytecodes(
                    self.codes.as_slice(),
                    gas_extra,
                )?);
                let _ = self.compiled.set(compiled.clone());
                compiled
            }
        })
    }

    pub fn into_array(self) -> Vec<u8> {
        self.codes.into_vec()
    }
}
