use field::{BytesW2, Decode, Encode, Reader, Uint1};

base::impl_fields_to_json!(CodeStuff { conf, data });

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct CodeStuff {
    pub conf: Uint1,
    pub data: BytesW2,
}

field::wire_struct_schema!(CodeStuff { conf: Uint1, data: BytesW2 });

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

impl Encode for CodeStuff {
    fn size(&self) -> usize {
        field::Encode::size(&self.conf) + field::Encode::size(&self.data)
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.conf.encode_to(out);
        self.data.encode_to(out);
    }
}

impl Decode for CodeStuff {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let conf = r.read()?;
        let data = r.read()?;
        Ok((Self { conf, data }, r.used()))
    }
}
