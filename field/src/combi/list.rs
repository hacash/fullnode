



#[macro_export] 
macro_rules! combi_list {
    ($class:ident, $cty:ty, $vty:ty) => (


#[derive(Default, Clone, PartialEq, Eq)]
pub struct $class  {
	count: $cty,
	lists: Vec<$vty>,
}

impl std::fmt::Debug for $class {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,"[list {}]", *self.count)
    }
}

impl std::ops::Index<usize> for $class {
    type Output = $vty;
    fn index(&self, idx: usize) -> &Self::Output {
        &self.lists[idx]
    }
}

impl std::ops::IndexMut<usize> for $class {
    fn index_mut(&mut self, idx: usize) -> &mut Self::Output{
        &mut self.lists[idx]
    }
}

impl Parse for $class {

    fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
        let mut seek = self.count.parse(buf)?;
        let count = *self.count as usize;
        self.lists = Vec::new();
        for _ in 0..count {
            let (obj, mvsk) = <$vty>::create(&buf[seek..])?;
            seek += mvsk;
            self.lists.push(obj);
        }
        Ok(seek)
    }

}


impl Serialize for $class {

    fn serialize(&self) -> Vec<u8> {
        let mut resdt = self.count.serialize();
        let count = *self.count as usize;
        for i in 0..count {
            let mut vardt = self.lists[i].serialize();
            resdt.append(&mut vardt);
        }
        resdt
    }

    fn size(&self) -> usize {
        let mut size = self.count.size();
        let count = *self.count as usize;
        for i in 0..count {
            size += self.lists[i].size();
        }
        size
    }

}

impl_field_only_new!{$class}

impl $class {

	pub fn length(&self) -> usize {
		*self.count as usize
	}

	pub fn count(&self) -> &$cty {
		&self.count
	}

	pub fn list(&self) -> &Vec<$vty> {
		&self.lists
	}

    pub fn replace(&mut self, i: usize, v: $vty) -> Rerr {
        let tl = self.length() as usize;
        if i >= tl {
            return errf!("list index overflow")
        }
        self.lists[i] = v;
        Ok(())
    }

	pub fn push(&mut self, v: $vty) -> Rerr {
        if *self.count as usize + 1 > <$cty>::MAX as usize {
            return errf!("append size overflow")
        }
		self.count += 1;
        self.lists.push(v);
        Ok(())
	}

	pub fn append(&mut self, mut list: Vec<$vty>) -> Rerr {
        if *self.count as usize + list.len() > <$cty>::MAX as usize {
            return errf!("append size overflow")
        }
		self.count += list.len() as u8;
        self.lists.append(&mut list);
        Ok(())
	}

	pub fn pop(&mut self) -> Option<$vty> {
        let n = *self.count;
        match n {
            0 => None,
            _ => {
                self.count -= 1u8;
                self.lists.pop()
            }
        }
	}

	pub fn as_mut(&mut self) -> &mut Vec<$vty> {
	    &mut self.lists
    }

}






	)
}



// test
combi_list!(TestFieldList9375649365, Uint1, Uint1);

