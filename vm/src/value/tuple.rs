#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct TupleItem {
    items: Rc<[Value]>,
}

impl TupleItem {
    fn read(&self) -> ReadList<'_> {
        ReadList::Slice(self.as_slice())
    }

    fn check_len(len: usize, max_len: usize) -> VmrtErr {
        if len == 0 {
            return itr_err_code!(CompoPackError);
        }
        if len > max_len {
            return itr_err_code!(OutOfCompoLen);
        }
        Ok(())
    }

    pub fn new(items: Vec<Value>) -> VmrtRes<Self> {
        if items.is_empty() {
            return itr_err_code!(CompoPackError);
        }
        // Runtime tuple length is enforced at VM entry/opcode sites; this
        // constructor stays cap-free for trusted rebuild paths.
        for item in &items {
            item.check_tuple_item()?;
        }
        Ok(Self {
            items: Rc::from(items.into_boxed_slice()),
        })
    }

    pub fn pack(cap: &SpaceCap, ops: &mut Stack) -> VmrtRes<(Value, usize)> {
        let n = ops.pop()?.extract_u16()? as usize;
        Self::check_len(n, cap.tuple_length)?;
        let items = ops.taken(n)?;
        Ok((Value::Tuple(Self::new(items)?), n))
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn as_slice(&self) -> &[Value] {
        &self.items
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.items, &other.items)
    }

    pub fn to_vec(&self) -> Vec<Value> {
        self.items.to_vec()
    }

    pub fn val_size(&self) -> usize {
        let mut sum = 0usize;
        for item in self.items.iter() {
            sum = add_size_saturating(sum, item.val_size());
            if sum == usize::MAX {
                break;
            }
        }
        sum
    }

    pub fn length(&self, cap: &SpaceCap) -> VmrtRes<Value> {
        let len = self.len();
        if len > cap.tuple_length {
            return itr_err_code!(OutOfCompoLen);
        }
        Ok(Value::U32(len as u32))
    }

    pub fn haskey(&self, k: Value) -> VmrtRes<Value> {
        self.read().haskey(k)
    }

    pub fn itemget(&self, k: Value) -> VmrtRes<Value> {
        self.read().itemget(k)
    }

    pub fn to_list_with_stats(&self, cap: &SpaceCap) -> VmrtRes<(Value, usize, usize)> {
        let len = self.items.len();
        if len > cap.compo_length {
            return itr_err_code!(OutOfCompoLen);
        }
        let mut bsz = 0usize;
        let mut items = std::collections::VecDeque::with_capacity(len);
        for item in self.items.iter().cloned() {
            bsz = add_size_saturating(bsz, item.val_size());
            items.push_back(item);
        }
        Ok((Value::Compo(CompoItem::list(items)?), len, bsz))
    }

    pub fn content_eq(&self, other: &Self) -> VmrtRes<bool> {
        if self.ptr_eq(other) {
            return Ok(true);
        }
        if self.len() != other.len() {
            return Ok(false);
        }
        for (lhs, rhs) in self.as_slice().iter().zip(other.as_slice().iter()) {
            if !value_content_eq(lhs, rhs)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn compare_fee(&self, other: &Self, container_header_fee: usize) -> usize {
        if self.ptr_eq(other) {
            return container_header_fee;
        }
        if self.len() != other.len() {
            return container_header_fee;
        }
        let mut fee = container_header_fee;
        for (lhs, rhs) in self.as_slice().iter().zip(other.as_slice().iter()) {
            fee = add_size_saturating(fee, value_compare_fee(lhs, rhs, container_header_fee));
            if fee == usize::MAX {
                break;
            }
        }
        fee
    }

    pub fn to_string(&self) -> String {
        let items: Vec<_> = self.items.iter().map(Value::to_string).collect();
        format!("tuple({})[{}]", self.items.len(), items.join(","))
    }

    pub fn to_json(&self) -> String {
        let items: Vec<_> = self.items.iter().map(Value::to_json).collect();
        format!("{{\"$tuple\":[{}]}}", items.join(","))
    }

    pub fn to_debug_json(&self) -> String {
        let items: Vec<_> = self.items.iter().map(Value::to_debug_json).collect();
        format!("{{\"$tuple\":[{}]}}", items.join(","))
    }
}
