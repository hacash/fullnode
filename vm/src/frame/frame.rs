





#[derive(Debug, Default)]
pub struct CallFrame {
    contract_count: usize,
    frames: Vec<Frame>,
}


impl CallFrame {

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn pop(&mut self) -> Option<Frame> {
        self.frames.pop()
    }

    pub fn push(&mut self, frame: Frame) {
        self.frames.push(frame);
    }

    pub fn increase(&mut self, r: &mut Resoure) -> VmrtRes<Frame> {
        let cap = &r.space_cap;
        if self.frames.len() >= cap.call_depth {
            return itr_err_code!(OutOfCallDepth)
        }
        Ok(match self.frames.last() {
            Some(f) => f.next(r),
            None => Frame::new(r),
        })
    }

    pub fn reclaim(mut self, r: &mut Resoure) {
        while let Some(frame) = self.pop() {
            frame.reclaim(r)
        }
    }
}



/***************************************/



#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct Frame {
    pub pc: usize,
    pub mode: CallMode,
    pub depth: isize,
    pub codes: Vec<u8>,
    pub oprnds: Stack,
    pub locals: Stack,
    pub heap: Heap,
    pub ctxadr: ContractAddress, 
    pub curadr: ContractAddress, 
}



impl Frame {

    pub fn reclaim(self, r: &mut Resoure) {
        r.stack_reclaim(self.oprnds);
        r.stack_reclaim(self.locals);
        r.heap_reclaim(self.heap);
    }

    pub fn new(r: &mut Resoure) -> Self {
        let mut f = Self{
            oprnds: r.stack_allocat(),
            locals: r.stack_allocat(),
            heap: r.heap_allocat(),
            ..Default::default()
        };
        let cap = &r.space_cap;
        f.oprnds.reset(cap.total_stack);
        f.locals.reset(cap.total_local);
        f.heap.reset(cap.max_heap_seg);
        f
    }

    pub fn next(&self, r: &mut Resoure) -> Self {
        let mut f = Self::new(r);
        let cap = &r.space_cap;
        f.oprnds.reset(cap.total_stack - self.oprnds.len());
        f.locals.reset(cap.total_local - self.locals.len());
        f.ctxadr = self.ctxadr.clone();
        f.curadr = self.curadr.clone();
        f.depth = self.depth + 1;
        f
    }

    pub fn pop_value(&mut self) -> VmrtRes<Value> {
        self.oprnds.pop()
    }

    pub fn push_value(&mut self, v: Value) -> VmrtErr {
        self.oprnds.push(v)
    }

    /*
        compile irnode
    */
    pub fn prepare(&mut self, mode: CallMode, codes: FnObj, param: Option<Value>) -> VmrtErr {
        use CodeType::*;
        self.pc = 0;
        self.mode = mode;
        self.codes = match codes.ctype {
            Bytecode => codes.into_array(),
            IRNode => runtime_irs_to_bytecodes(&codes.codes)?,
        };
        if let Some(p) = param {
            p.canbe_func_argv()?;
            self.oprnds.push(p)?; // param into stack
        }
        Ok(())
    }

    pub fn execute(&mut self, r: &mut Resoure, env: &mut ExecEnv) -> VmrtRes<CallExit> {
        execute_code(
            &mut self.pc,
            &self.codes,
            self.mode,
            self.depth,
            env.gas,
            &r.gas_table,
            &r.gas_extra,
            &r.space_cap,
            &mut self.oprnds,
            &mut self.locals,
            &mut self.heap,
            &mut r.global_vals,
            &mut r.memory_vals,
            env.ctx.as_ext_caller(),
            env.sta,
            &self.ctxadr,
            &self.curadr,
        )
    }

}

