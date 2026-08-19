use std::cell::{Ref, RefCell, RefMut};

#[derive(Debug, Clone)]
enum Compo {
    List(VecDeque<Value>),
    Map(BTreeMap<Vec<u8>, Value>),
}

impl PartialEq for Compo {
    fn eq(&self, _: &Self) -> bool {
        // Intentionally not VM semantic equality.
        // `Compo` lives behind `CompoItem(Rc<RefCell<_>>)` and Rust `==` must not be treated
        // as contract-visible value comparison. VM content equality is implemented separately via
        // `value_content_eq` / `CompoItem::content_eq`.
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
            return itr_err_code!(CompoOpOverflow);
        }
    };
}

impl Compo {
    fn to_string(&self) -> String {
        match self {
            Self::List(a) => {
                let sss: Vec<_> = a.iter().map(|a| a.to_string()).collect();
                format!("[{}]", sss.join(","))
            }
            Self::Map(b) => {
                let mmm: Vec<_> = b
                    .iter()
                    .map(|(k, v)| format!("0x{}:{}", k.to_hex(), v.to_string()))
                    .collect();
                format!("{{{}}}", mmm.join(","))
            }
        }
    }

    #[cfg(feature = "full")]
    fn to_json(&self) -> String {
        match self {
            Self::List(a) => {
                let sss: Vec<_> = a.iter().map(|a| a.to_json()).collect();
                format!("[{}]", sss.join(","))
            }
            Self::Map(b) => match Self::map_debug_json(b) {
                Some(s) => s,
                None => {
                    let mmm: Vec<_> = b
                        .iter()
                        .map(|(k, v)| format!(r#"{{"key_hex":"{}","value":{}}}"#, k.to_hex(), v.to_json()))
                        .collect();
                    format!(r#"{{"$map":[{}]}}"#, mmm.join(","))
                }
            }
        }
    }

    #[cfg(feature = "full")]
    fn to_debug_json(&self) -> String {
        match self {
            Self::List(a) => {
                let sss: Vec<_> = a.iter().map(Value::to_debug_json).collect();
                format!("[{}]", sss.join(","))
            }
            Self::Map(b) => match Self::map_debug_json(b) {
                Some(s) => s,
                None => {
                    let mmm: Vec<_> = b
                        .iter()
                        .map(|(k, v)| match bytes_try_to_readable_string(k) {
                            Some(s) => format!(
                                r#"{{"key":{},"key_hex":"{}","value":{}}}"#,
                                serde_json::to_string(&s).unwrap(),
                                k.to_hex(),
                                v.to_debug_json()
                            ),
                            None => format!(
                                r#"{{"key_hex":"{}","value":{}}}"#,
                                k.to_hex(),
                                v.to_debug_json()
                            ),
                        })
                        .collect();
                    format!(r#"{{"$map":[{}]}}"#, mmm.join(","))
                }
            },
        }
    }

    #[cfg(feature = "full")]
    fn map_debug_json(items: &BTreeMap<Vec<u8>, Value>) -> Option<String> {
        let mut mmm = Vec::with_capacity(items.len());
        for (k, v) in items {
            let key = bytes_try_to_readable_string(k)?;
            mmm.push(format!(
                "{}:{}",
                serde_json::to_string(&key).unwrap(),
                v.to_debug_json()
            ));
        }
        Some(format!("{{{}}}", mmm.join(",")))
    }

    fn len(&self) -> usize {
        match self {
            Self::List(a) => a.len(),
            Self::Map(b) => b.len(),
        }
    }

    fn val_size(&self) -> usize {
        match self {
            Self::List(items) => {
                let mut sum = 0usize;
                for v in items {
                    sum = add_size_saturating(sum, v.val_size());
                    if sum == usize::MAX {
                        break;
                    }
                }
                sum
            }
            Self::Map(items) => {
                let mut sum = 0usize;
                for (k, v) in items {
                    sum = add_size_saturating(sum, k.len());
                    if sum == usize::MAX {
                        break;
                    }
                    sum = add_size_saturating(sum, v.val_size());
                    if sum == usize::MAX {
                        break;
                    }
                }
                sum
            }
        }
    }

    pub fn clear(&mut self) {
        match self {
            Self::List(a) => a.clear(),
            Self::Map(b) => b.clear(),
        }
    }

    fn append(&mut self, cap: &SpaceCap, v: Value) -> VmrtErr {
        v.check_scalar()?;
        match self {
            Self::List(a) => {
                if a.len() >= cap.compo_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                a.push_back(v)
            }
            _ => ret_invalid_compo_op! {},
        }
        Ok(())
    }

    fn remove(&mut self, k: Value) -> VmrtErr {
        match self {
            Self::List(a) => {
                let i = k.extract_u32()?;
                if i as usize >= a.len() {
                    return itr_err_code!(CompoNoFindItem);
                }
                a.remove(i as usize);
            }
            Self::Map(b) => {
                let k = k.extract_key_bytes()?;
                if b.remove(&k).is_none() {
                    return itr_err_code!(CompoNoFindItem);
                }
            }
        }
        Ok(())
    }

    fn insert(&mut self, cap: &SpaceCap, k: Value, v: Value) -> VmrtErr {
        v.check_scalar()?;
        match self {
            Self::List(a) => {
                let i = k.extract_u32()?;
                checked_compo_op_len! {i, a};
                if a.len() >= cap.compo_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                a.insert(i as usize, v);
            }
            Self::Map(b) => {
                let k = k.extract_key_bytes()?;
                if !b.contains_key(&k) && b.len() >= cap.compo_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                b.insert(k, v);
            }
        }
        Ok(())
    }

    // return Bool
    fn haskey(&self, k: Value) -> VmrtRes<Value> {
        match self {
            Self::List(a) => ReadList::Deque(a).haskey(k),
            Self::Map(b) => {
                let k = k.extract_key_bytes()?;
                Ok(Value::Bool(b.contains_key(&k)))
            }
        }
    }

    fn itemget(&self, k: Value) -> VmrtRes<Value> {
        let v = match self {
            Self::List(a) => return ReadList::Deque(a).itemget(k),
            Self::Map(b) => {
                let nfer = || itr_err_code!(CompoNoFindItem);
                let k = k.extract_key_bytes()?;
                match b.get(&k) {
                    Some(b) => b.clone(),
                    _ => return nfer(), // error not find
                }
            }
        };
        Ok(v)
    }
}

/**********************************************************/

#[derive(Default, Clone)]
pub struct CompoItem {
    // VM DUP clones this Rc, so all aliases must observe later container writes.
    // A RefCell borrow conflict cannot be produced by valid bytecode: every guard
    // is local to one container operation and must not cross a reentrant VM call.
    // Use direct borrow()/borrow_mut() deliberately; a conflict is an implementation
    // invariant violation and must panic rather than produce a partial VM result.
    compo: Rc<RefCell<Compo>>,
}

#[cfg(feature = "full")]
impl Display for CompoItem {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "{}", self.to_json())
    }
}

impl Debug for CompoItem {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "{}", self.to_string())
    }
}

impl PartialEq for CompoItem {
    fn eq(&self, other: &Self) -> bool {
        // Intentionally pointer identity, not VM semantic equality.
        // This supports runtime/ref semantics and cheap identity checks. Any contract-visible
        // comparison must use `value_content_eq` / `CompoItem::content_eq` instead.
        self.ptr_eq(other)
    }
}

impl Eq for CompoItem {}

macro_rules! take_items_from_ops {
    ($is_map: expr, $cap: expr, $ops: expr) => {{
        let n = $ops.pop()?.extract_u16()? as usize;
        if n == 0 {
            return itr_err_code!(CompoPackError);
        }
        let mut max = $cap.compo_length;
        if $is_map {
            max *= 2; // for k => v
        }
        if n > max {
            return itr_err_code!(OutOfCompoLen);
        }
        let items = $ops.taken(n)?;
        items
    }};
}

impl CompoItem {
    pub fn to_string(&self) -> String {
        self.compo.borrow().to_string()
    }

    #[cfg(feature = "full")]
    pub fn to_json(&self) -> String {
        self.compo.borrow().to_json()
    }

    #[cfg(feature = "full")]
    pub fn to_debug_json(&self) -> String {
        self.compo.borrow().to_debug_json()
    }
}

impl CompoItem {
    pub fn list(l: VecDeque<Value>) -> VmrtRes<Self> {
        for item in &l {
            item.check_scalar()?;
        }
        Ok(Self {
            compo: Rc::new(RefCell::new(Compo::List(l))),
        })
    }

    pub fn map(m: BTreeMap<Vec<u8>, Value>) -> VmrtRes<Self> {
        for v in m.values() {
            v.check_scalar()?;
        }
        Ok(Self {
            compo: Rc::new(RefCell::new(Compo::Map(m))),
        })
    }

    pub fn pack_list(cap: &SpaceCap, ops: &mut Stack) -> VmrtRes<(Value, usize)> {
        let items = take_items_from_ops!(false, cap, ops);
        let len = items.len();
        for item in &items {
            item.check_scalar()?;
        }
        Ok((Value::Compo(Self::list(VecDeque::from(items))?), len))
    }

    pub fn pack_map(cap: &SpaceCap, ops: &mut Stack) -> VmrtRes<(Value, usize)> {
        let mut items: Vec<_> = take_items_from_ops!(true, cap, ops)
            .into_iter()
            .map(|a| Some(a))
            .collect();
        let m = items.len();
        if m % 2 != 0 {
            return itr_err_code!(CompoPackError); // map must k => v
        }
        let pair_count = m / 2;
        let mut mapobj = BTreeMap::new();
        for i in 0..pair_count {
            let k = items[i * 2].take().unwrap();
            let v = items[i * 2 + 1].take().unwrap();
            let k = k.extract_key_bytes()?;
            v.check_scalar()?;
            if mapobj.insert(k, v).is_some() {
                return itr_err_fmt!(CompoPackError, "duplicate key in pack_map");
            }
        }
        Ok((Value::Compo(Self::map(mapobj)?), m))
    }

    pub fn is_list(&self) -> bool {
        match &*self.compo.borrow() {
            Compo::List(..) => true,
            _ => false,
        }
    }

    pub fn is_map(&self) -> bool {
        match &*self.compo.borrow() {
            Compo::Map(..) => true,
            _ => false,
        }
    }

    pub fn list_ref(&self) -> VmrtRes<Ref<'_, VecDeque<Value>>> {
        let compo = self.compo.borrow();
        if !matches!(&*compo, Compo::List(..)) {
            return itr_err_code!(CompoOpNotMatch);
        }
        Ok(Ref::map(compo, |compo| match compo {
            Compo::List(list) => list,
            Compo::Map(..) => unreachable!(),
        }))
    }

    pub fn map_ref(&self) -> VmrtRes<Ref<'_, BTreeMap<Vec<u8>, Value>>> {
        let compo = self.compo.borrow();
        if !matches!(&*compo, Compo::Map(..)) {
            return itr_err_code!(CompoOpNotMatch);
        }
        Ok(Ref::map(compo, |compo| match compo {
            Compo::Map(map) => map,
            Compo::List(..) => unreachable!(),
        }))
    }

    fn list_mut(&self) -> VmrtRes<RefMut<'_, VecDeque<Value>>> {
        let compo = self.compo.borrow_mut();
        if !matches!(&*compo, Compo::List(..)) {
            return itr_err_code!(CompoOpNotMatch);
        }
        Ok(RefMut::map(compo, |compo| match compo {
            Compo::List(list) => list,
            Compo::Map(..) => unreachable!(),
        }))
    }

    #[allow(unused)]
    fn map_mut(&self) -> VmrtRes<RefMut<'_, BTreeMap<Vec<u8>, Value>>> {
        let compo = self.compo.borrow_mut();
        if !matches!(&*compo, Compo::Map(..)) {
            return itr_err_code!(CompoOpNotMatch);
        }
        Ok(RefMut::map(compo, |compo| match compo {
            Compo::Map(map) => map,
            Compo::List(..) => unreachable!(),
        }))
    }

    pub fn new_list() -> Self {
        Self {
            compo: Rc::new(RefCell::new(Compo::List(VecDeque::new()))),
        }
    }

    pub fn new_map() -> Self {
        Self {
            compo: Rc::new(RefCell::new(Compo::Map(BTreeMap::new()))),
        }
    }

    pub fn copy(&self) -> Self {
        self.copy_with_stats().0
    }

    pub fn copy_with_stats(&self) -> (Self, usize, usize) {
        let (data, len, bsz) = match &*self.compo.borrow() {
            Compo::List(src) => {
                let len = src.len();
                let mut bsz = 0usize;
                let mut list = VecDeque::with_capacity(len);
                for v in src.iter() {
                    bsz = add_size_saturating(bsz, v.val_size());
                    list.push_back(v.clone());
                }
                (Compo::List(list), len, bsz)
            }
            Compo::Map(src) => {
                let len = src.len();
                let mut bsz = 0usize;
                let mut map = BTreeMap::new();
                for (k, v) in src.iter() {
                    bsz = add_size_saturating(bsz, k.len());
                    bsz = add_size_saturating(bsz, v.val_size());
                    map.insert(k.clone(), v.clone());
                }
                (Compo::Map(map), len, bsz)
            }
        };
        (
            Self {
                compo: Rc::new(RefCell::new(data)),
            },
            len,
            bsz,
        )
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.compo, &other.compo)
    }

    pub fn merge(&mut self, cap: &SpaceCap, compo: CompoItem) -> VmrtErr {
        self.merge_with_stats(cap, compo).map(|_| ())
    }

    pub fn merge_with_stats(
        &mut self,
        cap: &SpaceCap,
        compo: CompoItem,
    ) -> VmrtRes<(usize, usize)> {
        if Rc::ptr_eq(&self.compo, &compo.compo) {
            return itr_err_code!(CompoOpInvalid);
        }
        match &mut *self.compo.borrow_mut() {
            Compo::List(l) => {
                let src = compo.list_ref()?.clone();
                let src_len = src.len();
                let new_len = l.len() + src_len;
                if new_len > cap.compo_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                let mut src_bsz = 0usize;
                for v in src.iter() {
                    v.check_scalar()?;
                    src_bsz = add_size_saturating(src_bsz, v.val_size());
                }
                l.extend(src);
                Ok((src_len, src_bsz))
            }
            Compo::Map(m) => {
                let src = compo.map_ref()?.clone();
                let src_len = src.len();
                let mut src_bsz = 0usize;
                for (k, v) in src.iter() {
                    v.check_scalar()?;
                    src_bsz = add_size_saturating(src_bsz, k.len());
                    src_bsz = add_size_saturating(src_bsz, v.val_size());
                    if m.contains_key(k) {
                        return itr_err_fmt!(CompoPackError, "duplicate key in merge");
                    }
                }
                let new_len = m.len() + src_len;
                if new_len > cap.compo_length {
                    return itr_err_code!(OutOfCompoLen);
                }
                for (k, v) in src {
                    m.insert(k, v);
                }
                Ok((src_len, src_bsz))
            }
        }
    }
}

impl CompoItem {
    pub fn len(&self) -> usize {
        self.compo.borrow().len()
    }

    pub fn val_size(&self) -> usize {
        self.compo.borrow().val_size()
    }

    pub fn length(&self, cap: &SpaceCap) -> VmrtRes<Value> {
        match &*self.compo.borrow() {
            Compo::List(a) => ReadList::Deque(a).length(cap),
            Compo::Map(b) => length_value_by_len(cap, b.len()),
        }
    }

    pub fn haskey(&self, k: Value) -> VmrtRes<Value> {
        self.compo.borrow().haskey(k)
    }

    pub fn remove(&mut self, k: Value) -> VmrtErr {
        let mut compo = self.compo.borrow_mut();
        compo.remove(k)
    }

    pub fn insert(&mut self, cap: &SpaceCap, k: Value, v: Value) -> VmrtErr {
        let mut compo = self.compo.borrow_mut();
        compo.insert(cap, k, v)
    }

    pub fn clear(&mut self) {
        let mut compo = self.compo.borrow_mut();
        compo.clear()
    }

    pub fn append(&mut self, cap: &SpaceCap, v: Value) -> VmrtErr {
        let mut compo = self.compo.borrow_mut();
        compo.append(cap, v)
    }

    pub fn itemget(&self, k: Value) -> VmrtRes<Value> {
        let compo = self.compo.borrow();
        compo.itemget(k)
    }

    pub fn keys(&self) -> VmrtRes<Value> {
        let map = self.map_ref()?;
        let keys = map.keys().map(|k| Value::Bytes(k.clone())).collect();
        Ok(Value::Compo(Self::list(keys)?))
    }

    pub fn keys_with_stats(&self) -> VmrtRes<(Value, usize, usize)> {
        let map = self.map_ref()?;
        let mut bsz = 0usize;
        let mut keys = VecDeque::with_capacity(map.len());
        for k in map.keys() {
            bsz = add_size_saturating(bsz, k.len());
            Value::Bytes(k.clone()).can_get_size()?;
            keys.push_back(Value::Bytes(k.clone()));
        }
        Ok((Value::Compo(Self::list(keys)?), map.len(), bsz))
    }

    pub fn values(&self) -> VmrtRes<Value> {
        let map = self.map_ref()?;
        let values = map.values().map(|v| v.clone()).collect();
        Ok(Value::Compo(Self::list(values)?))
    }

    pub fn values_with_stats(&self) -> VmrtRes<(Value, usize, usize)> {
        let map = self.map_ref()?;
        let mut bsz = 0usize;
        let mut values = VecDeque::with_capacity(map.len());
        for v in map.values() {
            bsz = add_size_saturating(bsz, v.val_size());
            values.push_back(v.clone());
        }
        Ok((Value::Compo(Self::list(values)?), map.len(), bsz))
    }

    pub fn content_eq(&self, other: &Self) -> VmrtRes<bool> {
        if self.ptr_eq(other) {
            return Ok(true);
        }
        match (self.list_ref(), other.list_ref()) {
            (Ok(lhs), Ok(rhs)) => {
                if lhs.len() != rhs.len() {
                    return Ok(false);
                }
                for (l, r) in lhs.iter().zip(rhs.iter()) {
                    if !value_content_eq(l, r)? {
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            (Err(_), Err(_)) => {}
            _ => return Ok(false),
        }

        let lhs = self.map_ref()?;
        let rhs = other.map_ref()?;
        if lhs.len() != rhs.len() {
            return Ok(false);
        }
        for (key, lval) in lhs.iter() {
            let Some(rval) = rhs.get(key) else {
                return Ok(false);
            };
            if !value_content_eq(lval, rval)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn compare_fee(&self, other: &Self, container_header_fee: usize) -> usize {
        if self.ptr_eq(other) {
            return container_header_fee;
        }
        match (self.list_ref(), other.list_ref()) {
            (Ok(lhs), Ok(rhs)) => {
                if lhs.len() != rhs.len() {
                    return container_header_fee;
                }
                let mut fee = container_header_fee;
                for (l, r) in lhs.iter().zip(rhs.iter()) {
                    fee = add_size_saturating(fee, value_compare_fee(l, r, container_header_fee));
                    if fee == usize::MAX {
                        break;
                    }
                }
                return fee;
            }
            (Err(_), Err(_)) => {}
            _ => return container_header_fee,
        }

        let Ok(lhs) = self.map_ref() else {
            return container_header_fee;
        };
        let Ok(rhs) = other.map_ref() else {
            return container_header_fee;
        };
        if lhs.len() != rhs.len() {
            return container_header_fee;
        }
        let mut fee = container_header_fee;
        for (key, lval) in lhs.iter() {
            fee = add_size_saturating(fee, key.len());
            if fee == usize::MAX {
                break;
            }
            let Some(rval) = rhs.get(key) else {
                break;
            };
            fee = add_size_saturating(fee, value_compare_fee(lval, rval, container_header_fee));
            if fee == usize::MAX {
                break;
            }
        }
        fee
    }

    pub fn take_first(&mut self) -> VmrtRes<Value> {
        let mut list = self.list_mut()?;
        match list.pop_front() {
            Some(v) => Ok(v),
            _ => itr_err_code!(CompoOpOverflow),
        }
    }

    /// Returns the last element of the list; remaining elements are discarded by the opcode.
    /// e.g. take_last([10, 20, 30]) -> 30
    pub fn take_last(&mut self) -> VmrtRes<Value> {
        let mut list = self.list_mut()?;
        match list.pop_back() {
            Some(v) => Ok(v),
            _ => itr_err_code!(CompoOpOverflow),
        }
    }
}
