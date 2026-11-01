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
    #[cfg(feature = "execute")]
    rv64im: Option<Arc<crate::calc::CalcFnObj>>,
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
            #[cfg(feature = "execute")]
            rv64im: None,
        })
    }

    /// Attach an already-verified calc kernel. The caller decoded the
    /// container once; this path does not decode or apply an admission cap.
    #[cfg(feature = "execute")]
    pub fn create_with_rv64im(
        fncnf: u8,
        pkg: CodePkg,
        agvty: Option<FuncArgvTypes>,
        rv64im: Arc<crate::calc::CalcFnObj>,
    ) -> VmrtRes<Self> {
        let ctype = pkg.code_type()?;
        if ctype != CodeType::RV64IM {
            return itr_err_code!(ItrErrCode::CodeTypeError);
        }
        Ok(Self {
            confs: fncnf,
            agvty,
            ctype,
            codes: ByteView::from_vec(pkg.data),
            compiled: Arc::new(std::sync::OnceLock::new()),
            rv64im: Some(rv64im),
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
            #[cfg(feature = "execute")]
            rv64im: None,
        }
    }

    #[cfg(feature = "execute")]
    pub fn rv64im_fn(&self) -> Option<&Arc<crate::calc::CalcFnObj>> {
        self.rv64im.as_ref()
    }

    pub const fn is_rv64im(&self) -> bool {
        matches!(self.ctype, CodeType::RV64IM)
    }

    #[cfg(feature = "execute")]
    pub fn exec_bytecodes(&self, _height: u64, gas_extra: &GasExtra) -> VmrtRes<ByteView> {
        use CodeType::*;
        match self.ctype {
            RV64IM => itr_err_code!(ItrErrCode::CodeTypeError),
            Bytecode => Ok(self.codes.clone()),
            IRNode => {
                if let Some(cached) = self.compiled.get() {
                    return Ok(cached.clone());
                }
                let compiled = ByteView::from_vec(crate::ir::runtime_irs_to_exec_bytecodes(
                    self.codes.as_slice(),
                    gas_extra,
                )?);
                let _ = self.compiled.set(compiled.clone());
                Ok(compiled)
            }
        }
    }

    pub fn into_array(self) -> Vec<u8> {
        self.codes.into_vec()
    }
}
