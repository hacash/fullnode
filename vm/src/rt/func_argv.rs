use super::*;
use crate::value::{TupleItem, Value, ValueTy};
use field::{Decode, Encode, FromJSON, JSONFormater, ToJSON, Uint1, json_unquote};
use sys::{Error, Ret, errf};

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct FuncArgvTypes {
    typnum: Uint1, // [4bit: output type, 4bit: inputs num]
    define: Vec<u8>,
}

impl FuncArgvTypes {
    pub fn new() -> Self {
        Self::default()
    }

    fn def_size(&self) -> usize {
        let n = bit4r!(self.typnum.uint()) as usize;
        (n + 1) / 2
    }

    pub fn param_count(&self) -> usize {
        bit4r!(self.typnum.uint()) as usize
    }

    pub fn check_output(&self, v: &mut Value) -> VmrtErr {
        let Some(oty) = self.output_type().map_ire(CallArgvTypeFail)? else {
            return Ok(());
        };
        if let Err(e) = v.cast_param(oty) {
            return itr_err_fmt!(CallArgvTypeFail, "check output failed: {:?}", e);
        }
        Ok(())
    }

    pub fn check_params(&self, v: &mut Value) -> VmrtErr {
        let ec = CallArgvTypeFail;
        let types = self.param_types().map_ire(ec)?;
        match types.as_slice() {
            [] => Ok(()),
            [ty] => v.cast_param(*ty),
            tys => {
                let Value::Tuple(tuple) = v else {
                    return itr_err_code!(CallArgvTypeFail);
                };
                let mut items = tuple.to_vec();
                if items.len() != tys.len() {
                    return itr_err_fmt!(
                        CallArgvTypeFail,
                        "param length invalid: expected {} but got {}",
                        tys.len(),
                        items.len()
                    );
                }
                for (item, ty) in items.iter_mut().zip(tys.iter().copied()) {
                    item.cast_param(ty)?;
                }

                *v = Value::Tuple(
                    TupleItem::new(items).map_err(|ItrErr(_, msg)| ItrErr::new(ec, &msg))?,
                );
                Ok(())
            }
        }
    }

    pub fn from_types(otp: Option<ValueTy>, tys: Vec<ValueTy>) -> Ret<Self> {
        let output_ty = match otp {
            Some(o) => {
                o.check_func_retv_type()?;
                (o as u8) << 4
            }
            _ => 0,
        };
        let n = tys.len();
        if n > crate::MAX_FUNC_PARAM_LEN {
            return errf!("func types cannot exceed {}", crate::MAX_FUNC_PARAM_LEN);
        }
        if n == 0 {
            return Ok(Self {
                typnum: Uint1::from(output_ty),
                define: vec![],
            });
        }
        let z = (n + 1) / 2;
        let mut dfs = vec![0u8; z];
        for (i, ty) in tys.into_iter().enumerate() {
            ty.check_func_argv_type()?;
            let ty = ty as u8;
            let tn = maybe!(i % 2 == 0, ty << 4, ty);
            dfs[i / 2] |= tn;
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
                ty.check_func_retv_type()?;
                Some(ty)
            }
        })
    }

    pub fn param_types(&self) -> Ret<Vec<ValueTy>> {
        let n = self.param_count();
        if n == 0 {
            return Ok(vec![]);
        }
        let mut tys = vec![ValueTy::Nil; n];
        let z = (n + 1) / 2;
        if z > self.define.len() {
            return errf!("FuncArgvTypes to bytes conversion error");
        }
        for (i, ty) in tys.iter_mut().enumerate() {
            let tn = self.define[i / 2];
            let t = maybe!(i % 2 == 0, bit4l!(tn), bit4r!(tn));
            let parsed = ValueTy::build(t)?;
            parsed.check_func_argv_type()?;
            *ty = parsed;
        }
        Ok(tys)
    }
}

impl Encode for FuncArgvTypes {
    fn size(&self) -> usize {
        1 + self.def_size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.typnum.encode_to(out);
        let z = self.def_size();
        let take = self.define.len().min(z);
        out.extend_from_slice(&self.define[..take]);
        if z > take {
            out.extend(std::iter::repeat_n(0u8, z - take));
        }
    }
}

impl Decode for FuncArgvTypes {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let (typnum, used) = Uint1::decode(buf)?;
        let mut out = Self {
            typnum,
            define: Vec::new(),
        };
        let z = out.def_size();
        if buf.len() < used + z {
            return Err(Error::decode("buffer too short for FuncArgvTypes"));
        }
        out.define.extend_from_slice(&buf[used..used + z]);
        Ok((out, used + z))
    }
}

impl ToJSON for FuncArgvTypes {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        format!("\"{}\"", hex::encode(self.encode()))
    }
}

impl FromJSON for FuncArgvTypes {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let data =
            hex::decode(json_unquote(json)).map_err(|_| Error::fault("cannot decode hex"))?;
        let (v, _) = FuncArgvTypes::decode(&data)?;
        *self = v;
        Ok(())
    }
}
