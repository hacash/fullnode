


#[derive(Debug, Default)]
pub struct Stack {
    pub datas: Vec<Value>,
    limit: usize, // max len
}


impl Stack {

    pub fn release(self) -> Vec<Value> {
        self.datas
    }

    pub fn new(lmt: usize) -> Stack {
        Stack {
            limit: lmt,
            ..Default::default()
        }
    }

    pub fn reset(&mut self, lmt: usize) {
        self.limit = lmt;
        self.datas.clear();
    }

    pub fn len(&self) -> usize {
        self.datas.len()
    }

    pub fn print_stack(&self) -> String {
        let mut prts = vec![];
        for d in &self.datas {
            prts.push(format!("{}", d));
        }
        format!("[{}]", prts.join(","))
    }
        
}



/*
* max size u16 = 65536 
*/
impl Stack {

    pub fn alloc(&mut self, num: u8) -> VmrtErr {
        let osz = self.datas.len();
        let tsz = osz + num as usize;
        if tsz >= self.limit {
            return itr_err_code!(OutOfStack)
        }
        self.datas.resize(tsz, Value::nil());
        Ok(())
    }

    #[inline(always)]
    pub fn peek<'a>(&'a mut self) -> VmrtRes<&'a mut Value> {
        let n = self.datas.len();
        if n <= 0 {
            return itr_err_fmt!(StackError, "Read empty stack")
        }
        Ok(unsafe { self.datas.get_unchecked_mut(n - 1) })
    }

    #[inline(always)]
    pub fn edit<'a>(&'a mut self, idx: u8) -> VmrtRes<&'a mut Value> {
        // let opt = mark > 5; // 0b00000111; (mark & 0b00011111)
        let idx = idx as usize;
        let n = self.datas.len();
        if idx > n {
            return itr_err_code!(OutOfStack)
        }
        Ok(unsafe { self.datas.get_unchecked_mut(idx) })
    }

    #[inline(always)]
    pub fn pop(&mut self) -> VmrtRes<Value> {
        self.datas.pop().ok_or_else(||ItrErr::new(StackError, "Pop empty stack"))
    }

    #[inline(always)]
    pub fn popn(&mut self, n: u8) -> VmrtRes<Vec<Value>> {
        let n = n as usize;
        if n == 0 {
            return Ok(vec![])
        }
        let cl = self.datas.len();
        if n > cl {
            return itr_err_fmt!(StackError, "pop empty stack")
        }
        let spx = cl - n;
        let res = self.datas.split_off(spx);
        Ok(res)
    }

    #[inline(always)]
    pub fn popx(&mut self, x: u8) -> VmrtErr {
        let x = x as usize;
        if x < 2 {
            return itr_err_fmt!(StackError, "inst popn param cannot less than 2")
        }
        let cl = self.datas.len();
        if x > cl {
            return itr_err_fmt!(StackError, "pop empty stack")
        }
        self.datas.truncate(cl - x);
        Ok(())

    }

    #[inline(always)]
    pub fn dupx(&mut self, x: u8) -> VmrtErr {
        let x = x as usize;
        let idx = self.datas.len() as i32 - x as i32 - 1;
        if idx < 0 {
            return itr_err_code!(OutOfStack)
        }
        self.push(self.datas[idx as usize].clone())?;
        Ok(())
    }

    #[inline(always)]
    pub fn reverse(&mut self) -> VmrtErr {
        let x = self.pop()?.checked_u8()? as usize;
        if x < 2 {
            return itr_err_fmt!(StackError, "inst reverse param cannot less than 2")
        }
        let mut list = Vec::new();
        for _ in 0..x {
            list.push(self.pop()?);
        }
        while let Some(a) = list.pop() {
            self.push(a)?;
        }
        Ok(())
    }

    /*
        return buf: b + a
    */
    pub fn cat(&mut self, cap: &SpaceCap) -> VmrtErr {
        let y = self.pop()?;
        let x = self.peek()?;
        *x = Value::concat(x, &y, cap)?;
        Ok(())
    }

    #[inline(always)]
    pub fn join(&mut self, cap: &SpaceCap) -> VmrtErr {
        let x = self.pop()?.checked_u8()? as usize;
        if x < 3 {
            return itr_err_fmt!(StackError, "inst join param cannot less than 3")
        }
        let mut value = Value::empty_bytes();
        for _ in 0..x {
            value = Value::concat(&self.pop()?, &value, cap)?;
        }
        self.push(value.valid(cap)?)
    }

    #[inline(always)]
    pub fn push(&mut self, it: Value) -> VmrtErr {
        if self.datas.len() >= self.limit {
            return itr_err_code!(OutOfStack)
        }
        self.datas.push(it);
        Ok(())
    }

    #[inline(always)]
    pub fn save(&mut self, idx: u16, it: Value) -> VmrtErr {
        let idx = idx as usize;
        if idx >= self.datas.len() {
            return itr_err_fmt!(LocalError, "Save local overflow")
        }
        self.datas[idx] = it;
        Ok(())
    }

    #[inline(always)]
    pub fn load(&self, idx: usize) -> VmrtRes<Value> {
        if idx >= self.datas.len() {
            return itr_err_fmt!(LocalError, "Read local overflow")
        }
        Ok(self.datas[idx].clone())
    }
    
    #[inline(always)]
    pub fn last(&self) -> VmrtRes<Value> {
        self.lastn(0)
    }

    #[inline(always)]
    pub fn lastn(&self, n: u16) -> VmrtRes<Value> {
        let n = n as usize;
        let l = self.datas.len();
        if n >= l {
            return itr_err_fmt!(StackError, "Read stack overflow")
        }
        Ok(self.datas[l-n-1].clone())
    }

    #[inline(always)]
    pub fn swap(&mut self) -> VmrtErr {
        let l = self.datas.len();
        if l < 2 {
            return itr_err_fmt!(StackError, "Read empty stack")
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