
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct FuncArgvTypes {
    typnum: Uint1, // [ 4bit: output type, 4bit: inputs num]
    define: Vec<u8>,
}

impl FuncArgvTypes {

    fn def_size(&self) -> usize {
        let n = bit4r!(self.typnum.uint()) as usize;
        match n {
            0 => 0,
            _ => n / 2 + 1
        }
    }

    pub fn check_output(&self, v: &mut Value) -> VmrtErr {
        let Some(oty) = self.output_type().map_ire(CallArgvTypeFail)? else {
            return Ok(())
        };
        if let Err(e) = v.checked_param_cast(oty) {
            return itr_err_fmt!(CallArgvTypeFail, "check output failed: {:?}", e);
        }
        // pass
        Ok(())
    }


    pub fn check_params(&self, v: &mut Value) -> VmrtErr {
        let ec = CallArgvTypeFail;
        // let err = |t1, t2| itr_err_fmt!(ec, "need {:?} but got {:?}", t1, t2);
        let types = self.param_types().map_ire(ec)?;
        let tn = types.len();
        match tn {
            // do not check
            0 => Ok(()),
            // check one argv
            1 => v.checked_param_cast(types[0]),
            // check list
            _ => {
                let vs = v.compo()?.list_mut()?;
                let vn = vs.len();
                if tn != vn {
                    return itr_err_fmt!(ec, "param length error need {} but got {}", tn, vn)
                }
                for i in 0..vn {
                    vs[i].checked_param_cast(types[i])?;
                }
                // all pass
                Ok(())
            }
        }
    }

    pub fn from_types(otp: Option<ValueTy>, tys: Vec<ValueTy>) -> Ret<Self> {
        let output_ty = match otp {
            Some(o) => { o.canbe_argv()?; (o as u8) << 4}
            _ => 0,
        };
        let n = tys.len();
        if n > 15 {
            return errf!("func types cannot more than 15")
        }
        if 0 == n {
            return Ok(Self{
                typnum: Uint1::from(output_ty),
                define: vec![],
            })
        }
        let z = n / 2 + 1;
        let mut dfs = vec![0u8; z];
        for i in 0..n {
            let ty = tys[i]; 
            ty.canbe_argv()?;
            let ty = ty as u8;
            let tn = maybe!( i % 2 == 0, ty << 4, ty);
            dfs[i/2] = dfs[i/2] | tn; 
        }
        let typnum = output_ty | (n as u8);
        Ok(Self {
            typnum: Uint1::from(typnum),
            define: dfs,
        })
    }

    pub fn output_type(&self) -> Ret<Option<ValueTy>> {
        let tn = bit4l!(self.typnum.uint());
        let ty = ValueTy::build(tn)?;
        Ok(match ty {
            ValueTy::Nil => None,
            _ => {
                ty.canbe_argv()?;
                Some(ty)
            }
        })
    }

    pub fn param_types(&self) -> Ret<Vec<ValueTy>> {
        let n = bit4r!(self.typnum.uint()) as usize;
        if 0 == n {
            return Ok(vec![])
        }
        let mut tys = vec![ValueTy::Nil; n];
        let z = n / 2 + 1;
        if z > self.define.len() {
            return errf!("FuncArgvTypes to bytes error")
        }
        for i in 0..n {
            let tn = self.define[i/2];
            let t = maybe!( i % 2 == 0, bit4l!(tn), bit4r!(tn) );
            let ty = ValueTy::build(t)?;
            ty.canbe_argv()?;
            tys[i] = ty;
        }
        Ok(tys)
    }

}

impl Parse for FuncArgvTypes {
    fn parse(&mut self, mut buf: &[u8]) -> Ret<usize> {
        self.typnum.parse(buf)?;
        buf = &buf[1..];
        let z =  self.def_size();
        self.define = bufeat(buf, z)?;
        Ok(1 + z)
    }
}

impl Serialize for FuncArgvTypes {
    fn serialize(&self) -> Vec<u8> {
        let z = self.def_size();
        let nvs = self.typnum.serialize();
        vec![nvs,
            self.define[0..z].to_vec(),
        ].concat()
    }
    fn size(&self) -> usize {
        1 + self.def_size()
    }
}

impl_field_only_new!{FuncArgvTypes}








