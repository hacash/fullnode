use field::{BytesW2, Uint1};

#[derive(Debug, Clone, PartialEq, Eq, field::FieldCodec)]
pub struct CodeStuff {
    pub conf: Uint1,
    pub data: BytesW2,
}

impl CodeStuff {
    pub fn parse_conf(&self) -> VmrtRes<CodeConf> {
        CodeConf::parse(self.conf.uint())
    }
}

impl TryFrom<&CodeStuff> for CodePkg {
    type Error = ItrErr;

    fn try_from(src: &CodeStuff) -> Result<Self, Self::Error> {
        let conf = src.parse_conf()?.raw();
        Ok(Self {
            conf,
            data: src.data.to_vec(),
        })
    }
}

impl TryFrom<CodeStuff> for CodePkg {
    type Error = ItrErr;

    fn try_from(src: CodeStuff) -> Result<Self, Self::Error> {
        let conf = src.parse_conf()?.raw();
        Ok(Self {
            conf,
            data: src.data.into_vec(),
        })
    }
}

impl TryFrom<&CodePkg> for CodeStuff {
    type Error = ItrErr;

    fn try_from(src: &CodePkg) -> Result<Self, Self::Error> {
        let conf = CodeConf::parse(src.conf)?.raw();
        Ok(Self {
            conf: Uint1::from(conf),
            data: BytesW2::from(src.data.clone()).map_ire(ItrErrCode::CastParamFail)?,
        })
    }
}

impl TryFrom<CodePkg> for CodeStuff {
    type Error = ItrErr;

    fn try_from(src: CodePkg) -> Result<Self, Self::Error> {
        let conf = CodeConf::parse(src.conf)?.raw();
        Ok(Self {
            conf: Uint1::from(conf),
            data: BytesW2::from(src.data).map_ire(ItrErrCode::CastParamFail)?,
        })
    }
}
