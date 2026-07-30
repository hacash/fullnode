use crate::rt::ItrErrCode::*;
use crate::rt::*;
use crate::value::*;

#[derive(Debug, Default)]
pub struct Stack {
    pub datas: Vec<Value>,
    limit: usize, // max len
}

impl Stack {
    pub fn release(self) -> Vec<Value> {
        self.datas
    }

    pub fn clear(&mut self) {
        self.datas.clear();
    }

    pub fn new(lmt: usize) -> Stack {
        Stack {
            limit: lmt,
            ..Default::default()
        }
    }

    pub fn reset(&mut self, lmt: usize) {
        self.limit = lmt;
        self.clear();
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn len(&self) -> usize {
        self.datas.len()
    }

    pub fn print_stack(&self) -> String {
        let mut text = String::from("[");
        for (i, d) in self.datas.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            text.push_str(&d.to_string());
        }
        text.push(']');
        text
    }
}

/* * max size u16 = 65536 */
impl Stack {
    #[inline(always)]
    fn pop_empty() -> ItrErr {
        ItrErr::new(StackError, "pop empty stack")
    }

    #[inline(always)]
    fn split_tail(&mut self, n: usize) -> Option<Vec<Value>> {
        let m = self.datas.len();
        if n > m {
            return None;
        }
        Some(self.datas.split_off(m - n))
    }

    #[inline(always)]
    fn get_mut_at(&mut self, idx: usize) -> VmrtRes<&mut Value> {
        self.datas
            .get_mut(idx)
            .ok_or_else(|| ItrErr::code(OutOfStack))
    }

    #[inline(always)]
    fn get_at(&self, idx: usize) -> VmrtRes<&Value> {
        self.datas.get(idx).ok_or_else(|| ItrErr::code(OutOfStack))
    }

    pub fn alloc(&mut self, num: u8) -> VmrtRes<u8> {
        let osz = self.datas.len();
        let tsz = osz
            .checked_add(num as usize)
            .ok_or_else(|| ItrErr::code(OutOfStack))?;
        if tsz > self.limit {
            return itr_err_code!(OutOfStack);
        }
        self.datas.resize(tsz, Value::nil());
        Ok(num)
    }

    #[inline(always)]
    pub fn peek<'a>(&'a mut self) -> VmrtRes<&'a mut Value> {
        self.datas
            .last_mut()
            .ok_or_else(|| ItrErr::new(StackError, "Read empty stack"))
    }

    #[inline(always)]
    pub fn peek_with_size<'a>(&'a mut self) -> VmrtRes<(&'a mut Value, usize)> {
        let v = self.peek()?;
        let sz = v.val_size();
        Ok((v, sz))
    }

    /// `from_top == 0` is stack top. Returns `None` when out of range.
    #[inline(always)]
    pub fn tail(&self, from_top: usize) -> Option<&Value> {
        let l = self.datas.len();
        (from_top < l).then(|| &self.datas[l - 1 - from_top])
    }

    /// `val_size()` of the stack value `from_top` slots below the top.
    #[inline(always)]
    pub fn tail_val_size(&self, from_top: usize) -> VmrtRes<usize> {
        self.tail(from_top)
            .map(Value::val_size)
            .ok_or_else(|| ItrErr::code(OutOfStack))
    }

    /// Sum `val_size()` of the top `n` values.
    #[inline(always)]
    pub fn tail_val_size_sum(&self, n: usize) -> VmrtRes<usize> {
        let l = self.datas.len();
        if n > l {
            itr_err_code!(OutOfStack)
        } else {
            Ok(self.datas[l - n..].iter().map(|v| v.val_size()).sum())
        }
    }

    pub fn compo<'a>(&'a mut self) -> VmrtRes<&'a mut CompoItem> {
        let pk = self.peek()?;
        let Some(compo) = pk.match_compo_mut() else {
            return itr_err_code!(CompoOpNotMatch);
        };
        Ok(compo)
    }

    #[inline(always)]
    pub fn slot_mut<'a>(&'a mut self, idx: u8) -> VmrtRes<&'a mut Value> {
        self.get_mut_at(idx as usize)
    }

    pub fn taken(&mut self, n: usize) -> VmrtRes<Vec<Value>> {
        self.split_tail(n).ok_or_else(Self::pop_empty)
    }

    #[inline(always)]
    pub fn pop(&mut self) -> VmrtRes<Value> {
        self.datas.pop().ok_or_else(Self::pop_empty)
    }

    #[inline(always)]
    pub fn popn(&mut self, n: u8) -> VmrtRes<Vec<Value>> {
        let n = n as usize;
        if n == 0 {
            return Ok(vec![]);
        }
        self.split_tail(n).ok_or_else(Self::pop_empty)
    }

    #[inline(always)]
    pub fn dupn(&mut self, n: u8) -> VmrtErr {
        let n = n as usize;
        if n < 2 {
            return itr_err_fmt!(StackError, "inst dupn param must be at least 2");
        }
        let m = self.datas.len();
        if n > m {
            return itr_err_fmt!(StackError, "dupn length overflow");
        }
        let s = m - n;
        for i in s..m {
            self.push(self.datas[i].clone())?;
        }
        Ok(())
    }

    #[inline(always)]
    pub fn roll(&mut self, x: u8) -> VmrtErr {
        let idx = self
            .datas
            .len()
            .checked_sub(x as usize + 1)
            .ok_or_else(|| ItrErr::code(OutOfStack))?;
        let item = self.datas.remove(idx);
        self.push(item)?;
        Ok(())
    }

    #[inline(always)]
    pub fn reverse(&mut self, x: u8) -> VmrtErr {
        let x = x as usize;
        if x < 2 {
            return itr_err_fmt!(StackError, "inst reverse param must be at least 2");
        }
        let l = self.datas.len();
        if x > l {
            return itr_err_fmt!(StackError, "reverse exceeds stack length");
        }
        self.datas[l - x..l].reverse();
        Ok(())
    }

    /* return buf: a + b */
    pub fn cat(&mut self, cap: &SpaceCap) -> VmrtErr {
        let y = self.pop()?;
        let x = self.peek()?;
        *x = Value::concat(x, &y, cap)?;
        Ok(())
    }

    #[inline(always)]
    pub fn join(&mut self, n: u8, cap: &SpaceCap) -> VmrtErr {
        let n = n as usize;
        if n < 3 {
            return itr_err_fmt!(StackError, "inst join param must be at least 3");
        }
        if n > self.datas.len() {
            return itr_err_fmt!(StackError, "pop empty stack");
        }
        let mut total = 0usize;
        for v in &self.datas[self.datas.len() - n..] {
            let blen = v.extract_bytes_len_with_error_code(BytesHandle)?;
            total = checked_value_output_add(cap, total, blen)?;
        }
        self.join_with_total(n as u8, total, cap)
    }

    #[inline(always)]
    pub fn join_with_total(&mut self, n: u8, total: usize, cap: &SpaceCap) -> VmrtErr {
        let n = n as usize;
        if n < 3 {
            return itr_err_fmt!(StackError, "inst join param must be at least 3");
        }
        if n > self.datas.len() {
            return itr_err_fmt!(StackError, "pop empty stack");
        }
        let total = checked_value_output_len(cap, total)?;
        let items = self.popn(n as u8)?;
        let mut data = Vec::with_capacity(total);
        for v in items {
            data.extend_from_slice(
                &v.extract_bytes()
                    .map_err(|ItrErr(_, msg)| ItrErr::new(BytesHandle, &msg))?,
            );
        }
        self.push(Value::bytes(data).valid(cap)?)
    }

    #[inline(always)]
    pub fn push(&mut self, it: Value) -> VmrtErr {
        if self.datas.len() >= self.limit {
            return itr_err_code!(OutOfStack);
        }
        self.datas.push(it);
        Ok(())
    }

    #[inline(always)]
    pub fn save(&mut self, idx: u16, it: Value) -> VmrtErr {
        *self.get_mut_at(idx as usize)? = it;
        Ok(())
    }

    #[inline(always)]
    pub fn load(&self, idx: usize) -> VmrtRes<Value> {
        Ok(self.get_at(idx)?.clone())
    }

    #[inline(always)]
    pub fn last(&self) -> VmrtRes<Value> {
        self.lastn(0)
    }

    #[inline(always)]
    pub fn lastn(&self, n: u16) -> VmrtRes<Value> {
        let n = n as usize;
        let idx = self
            .datas
            .len()
            .checked_sub(n + 1)
            .ok_or_else(|| ItrErr::new(StackError, "Read stack underflow"))?;
        Ok(self.get_at(idx)?.clone())
    }

    #[inline(always)]
    pub fn swap(&mut self) -> VmrtErr {
        let l = self.datas.len();
        if l < 2 {
            return itr_err_fmt!(StackError, "Read empty stack");
        }
        let a = l - 1;
        let b = l - 2;
        self.datas.swap(a, b);
        Ok(())
    }

    #[inline(always)]
    pub fn append(&mut self, mut vs: Vec<Value>) -> VmrtErr {
        let s = vs.len();
        if s + self.datas.len() > self.limit {
            return itr_err_code!(OutOfStack);
        }
        self.datas.append(&mut vs);
        Ok(())
    }
}
