
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct FuncArgvTypes {
    output: Uint1, // output
    number: Uint1, // inputs num
    define: Vec<u8>,
}

impl FuncArgvTypes {

    fn def_size(&self) -> usize {
        let n = self.number.uint() as usize;
        match n {
            0 => 0,
            _ => n / 2 + 1
        }
    }

    pub fn check_output(&self, v: &Value) -> VmrtErr {
        let Some(oty) = map_err_itr!(CallArgvTypeFail, self.output_type())? else {
            return Ok(())
        };
        if v.ty() != oty {
            return itr_err_code!(CallArgvTypeFail);
        }
        // pass
        Ok(())
    }


    pub fn check_params(&self, v: &Value) -> VmrtErr {
        let err = || itr_err_code!(CallArgvTypeFail);
        let types = map_err_itr!(CallArgvTypeFail, self.param_types())?;
        let tn = types.len();
        match tn {
            // do not check
            0 => Ok(()),
            // check one argv
            1 => maybe!(v.ty()==types[0], Ok(()), err() ),
            // check list
            _ => {
                let vs = v.compo_ref()?.list_ref()?;
                let vn = vs.len();
                if tn != vn {
                    return err()
                }
                for i in 0..vn {
                    if vs[i].ty() != types[i] {
                        return err()
                    }
                }
                // all pass
                Ok(())
            }
        }
    }

    pub fn from_types(otp: ValueTy, tys: Vec<ValueTy>) -> Ret<Self> {
        otp.canbe_argv()?;
        let output = Uint1::from(otp as u8);
        let n = tys.len();
        if n > 200 {
            return errf!("func types cannot more than 200")
        }
        if 0 == n {
            return Ok(Self{
                output,
                number: Uint1::from(0),
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
        Ok(Self {
            output,
            number: Uint1::from(n as u8),
            define: dfs,
        })
    }

    pub fn output_type(&self) -> Ret<Option<ValueTy>> {
        let ty = ValueTy::build(self.output.uint())?;
        Ok(match ty {
            ValueTy::Nil => None,
            _ => Some(ty),
        })
    }

    pub fn param_types(&self) -> Ret<Vec<ValueTy>> {
        let n = self.number.uint() as usize;
        if 0 == n {
            return Ok(vec![])
        }
        let mut tys = vec![ValueTy::Nil; n];
        let z = n / 2 + 1;
        if z >= self.define.len() {
            return errf!("FuncArgvTypes to bytes error")
        }
        for i in 0..n {
            let tn = self.define[i/2];
            let t = match i % 2 == 0 {
                true  => tn >> 4,
                false => tn & 0b00001111,
            };
            tys[i] = ValueTy::build(t)?;
        }
        Ok(tys)
    }

}

impl Parse for FuncArgvTypes {
    fn parse(&mut self, mut buf: &[u8]) -> Ret<usize> {
        self.output.parse(buf)?;
        buf = &buf[1..];
        self.number.parse(buf)?;
        buf = &buf[1..];
        let z =  self.def_size();
        self.define = bufeat(buf, z)?;
        Ok(2 + z)
    }
}

impl Serialize for FuncArgvTypes {
    fn serialize(&self) -> Vec<u8> {
        let z = self.def_size();
        vec![
            self.output.serialize(),
            self.number.serialize(),
            self.define[0..z].to_vec(),
        ].concat()
    }
    fn size(&self) -> usize {
        1 + 1 + self.def_size()
    }
}

impl_field_only_new!{FuncArgvTypes}








