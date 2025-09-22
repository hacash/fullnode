
#[derive(Debug, Clone)]
enum Compo {
    List(VecDeque<Value>),
    Map(HashMap<Vec<u8>, Value>),
}

impl PartialEq for Compo {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Eq for Compo {}

impl Default for Compo {
    fn default() -> Self {
        Self::List(VecDeque::new())
    }
}


macro_rules! ret_invalid_compo_op {
    () => {
        return itr_err_code!(CompoOpInvalid)
    };
}


macro_rules! checked_compo_op_len {
    ($i:expr, $a: expr) => {
        if $i as usize > $a.len() {
            return itr_err_code!(CompoOpOverflow)
        }
    };
}

impl Compo {

    fn len(&self) -> usize {
        match self {
            Self::List(a) => a.len(),
            Self::Map(b)  => b.len(),
        }
    }

    pub fn clear(&mut self) {
        match self {
            Self::List(a) => a.clear(),
            Self::Map(b)  => b.clear(),
        }
    }

    fn append(&mut self, v: Value) -> VmrtErr {
        v.canbe_value()?;
        match self {
            Self::List(a) => a.push_back(v),
            _ => ret_invalid_compo_op!{},
        }
        Ok(())
    }

    fn remove(&mut self, k: Value) -> VmrtErr {
        match self {
            Self::List(a) => {
                let i = k.checked_u32()?;
                a.remove(i as usize);
            }
            Self::Map(b) => {
                let k = k.canbe_key()?;
                b.remove(&k);
            }
        }
        Ok(())
    }

    fn insert(&mut self, k: Value, v: Value) -> VmrtErr {
        v.canbe_value()?;
        match self {
            Self::List(a) => {
                let i = k.checked_u32()?;
                checked_compo_op_len!{i, a};
                a.insert(i as usize, v);
            }
            Self::Map(b) => {
                let k = k.canbe_key()?;
                b.insert(k, v);
            }
        }
        Ok(())
    }

    // return Bool
    fn haskey(&self, k: Value) -> VmrtRes<Value> {
        let hsk = match self {
            Self::List(a) => {
                let i = k.checked_u32()? as usize;
                i < a.len()
            }
            Self::Map(b) => {
                let k = k.canbe_key()?;
                b.contains_key(&k)
            }
        };
        Ok(Value::Bool(hsk))
    }

    fn find(&mut self, k: Value) -> VmrtRes<Value> {
        let v = match self {
            Self::List(a) => {
                let i = k.checked_u32()?;
                match a.get(i as usize) {
                    Some(a) => a.clone(),
                    _ => Value::Nil,
                }
            }
            Self::Map(b) => {
                let k = k.canbe_key()?;
                match b.get(&k) {
                    Some(b) => b.clone(),
                    _ => Value::Nil,
                }
            }
        };
        Ok(v)
    }


}




/**********************************************************/




#[derive(Default, Debug, Clone)]
pub struct CompoItem {
    compo: Rc<UnsafeCell<Compo>>,
}

impl PartialEq for CompoItem {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Eq for CompoItem {}


macro_rules! get_compo_inner_ref {
    ($self: ident) => {
        unsafe { &*$self.compo.get() }
    };
}

macro_rules! get_compo_inner_mut {
    ($self: ident) => {
        unsafe { &mut *$self.compo.get() }
    };
}


impl CompoItem {

    pub fn list(l: VecDeque<Value>) -> Self {
        Self {
            compo: Rc::new(UnsafeCell::new(Compo::List(l))),
        }
    }

    pub fn map(m: HashMap<Vec<u8>, Value>) -> Self {
        Self {
            compo: Rc::new(UnsafeCell::new(Compo::Map(m))),
        }
    }

    pub fn pack_list(_ops: &mut Stack) -> VmrtRes<Value> {
        unimplemented!()
    }

    pub fn pack_map(_ops: &mut Stack) -> VmrtRes<Value> {
        unimplemented!()
    }

    pub fn is_list(&self) -> bool {
        match get_compo_inner_ref!(self) {
            Compo::List(..) => true,
            _ => false,
        }
    }

    pub fn is_map(&self) -> bool {
        match get_compo_inner_ref!(self) {
            Compo::Map(..) => true,
            _ => false,
        }
    }

    pub fn list_ref(&self) -> VmrtRes<&VecDeque<Value>> {
        let r = get_compo_inner_ref!(self);
        let Compo::List(list) = r else {
            return itr_err_code!(CompoOpNotMatch)
        };
        Ok(list)
    }


    pub fn new_list() -> Self {
        Self {
            compo: Rc::new(UnsafeCell::new(Compo::List(VecDeque::new()))),
        }
    }

    pub fn new_map() -> Self {
        Self {
            compo: Rc::new(UnsafeCell::new(Compo::Map(HashMap::new()))),
        }
    }

    pub fn copy(&self) -> Self {
        let data = get_compo_inner_ref!(self).clone();
        Self {
            compo: Rc::new(UnsafeCell::new(data)),
        }
    }

    pub fn merge(&mut self, _compo: CompoItem) -> VmrtErr {
        unimplemented!()
    }


}


macro_rules! checked_compo_length {
    ($compo: expr, $cap: expr) => { {
        let n = $compo.len();
        if n > $cap.max_compo_length {
            return itr_err_code!(OutOfCompoLen)
        }
        n
    } };
}


impl CompoItem {

    pub fn len(&self) -> usize {
        get_compo_inner_ref!(self).len()
    }

    pub fn length(&self, cap: &SpaceCap) -> VmrtRes<Value> {
        let n = checked_compo_length!{get_compo_inner_ref!(self), cap};
        Ok(Value::U32(n as u32))
    }

    pub fn haskey(&self, k: Value) -> VmrtRes<Value> {
        get_compo_inner_ref!(self).haskey(k)
    }

    pub fn remove(&mut self, k: Value) -> VmrtErr {
        let compo = get_compo_inner_mut!(self);
        compo.remove(k)
    }

    pub fn insert(&mut self, cap: &SpaceCap, k: Value, v: Value) -> VmrtErr {
        let compo = get_compo_inner_mut!(self);
        compo.insert(k, v)?;
        checked_compo_length!{compo, cap};
        Ok(())
    }

    pub fn clear(&mut self) {
        let compo = get_compo_inner_mut!(self);
        compo.clear()
    }

    pub fn append(&mut self, cap: &SpaceCap, v: Value) -> VmrtErr {
        let compo = get_compo_inner_mut!(self);
        compo.append(v)?;
        checked_compo_length!{compo, cap};
        Ok(())
    }

    pub fn find(&self, k: Value) -> VmrtRes<Value> {
        let compo = get_compo_inner_mut!(self);
        compo.find(k)
    }

    pub fn keys(&mut self) -> VmrtErr {
        unimplemented!()
    }

    pub fn values(&mut self) -> VmrtErr {
        unimplemented!()
    }

    pub fn head(&mut self) -> VmrtErr {
        unimplemented!()
    }

    pub fn tail(&mut self) -> VmrtErr {
        unimplemented!()
    }




}











