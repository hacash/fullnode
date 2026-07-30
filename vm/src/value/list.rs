
pub enum ReadList<'a> {
    Slice(&'a [Value]),
    Deque(&'a VecDeque<Value>),
}

impl ReadList<'_> {
    #[inline(always)]
    pub fn len(&self) -> usize {
        match self {
            Self::Slice(items) => items.len(),
            Self::Deque(items) => items.len(),
        }
    }

    #[inline(always)]
    fn get(&self, idx: usize) -> Option<&Value> {
        match self {
            Self::Slice(items) => items.get(idx),
            Self::Deque(items) => items.get(idx),
        }
    }

    #[inline(always)]
    pub fn length(&self, cap: &SpaceCap) -> VmrtRes<Value> {
        length_value_by_len(cap, self.len())
    }

    #[inline(always)]
    pub fn haskey(&self, k: Value) -> VmrtRes<Value> {
        let i = k.extract_u32()? as usize;
        Ok(Value::Bool(i < self.len()))
    }

    #[inline(always)]
    pub fn itemget(&self, k: Value) -> VmrtRes<Value> {
        let i = k.extract_u32()? as usize;
        match self.get(i) {
            Some(v) => Ok(v.clone()),
            None => itr_err_code!(CompoNoFindItem),
        }
    }
}
