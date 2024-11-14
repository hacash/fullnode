

#[macro_export]
macro_rules! combi_dynvec {
    ($class:ident, $lenty:ty, $dynty:ident, $parseobjfunc: path) => (


#[derive(Default, Clone)]
pub struct $class {
    count: $lenty,
    vlist: Vec<Box<dyn $dynty>>
}

impl std::fmt::Debug for $class {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,"[dyn list {}]", self.count.to_u64())
    }
}

impl PartialEq for $class {
    #[inline]
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Eq for $class {}


impl Parse for $class {

    fn parse(&mut self, buf: &[u8]) -> Ret<usize> {
        let mut seek = 0;
        let count = *self.count as usize;
        self.vlist = Vec::new();
        for _ in 0..count {
            let(obj, mvsk) = $parseobjfunc(&buf[seek..]) ?;
            seek += mvsk;
            self.vlist.push(obj);
        }
        Ok(seek)
    }
}

impl Serialize for $class {
    
    fn serialize(&self) -> Vec<u8> {
        let mut bts = vec![];
        for v in &self.vlist {
            bts.push(v.serialize());
        }
        bts.concat()
    }

    fn size(&self) -> usize {
        let mut sznum = 0;
        for v in &self.vlist {
            sznum += v.size();
        }
        sznum
    }

}

impl_field_only_new!{$class}

impl $class {

	pub fn push(&mut self, v: Box<dyn $dynty>) -> Rerr {
        if *self.count >= <$lenty>::MAX {
            return errf!("list length overflow");
        }
		self.count += 1u8;
        self.vlist.push(v);
        Ok(())
	}

	pub fn pop(&mut self) -> Option<Box<dyn $dynty>> {
        let n = self.count.to_uint();
        match n {
            0 => None,
            _ => {
                self.count -= 1u8;
                self.vlist.pop()
            }
        }
	}

	pub fn count(&self) -> &$lenty {
		&self.count
	}

	pub fn set_count(&mut self, c: $lenty) {
		self.count = c;
	}

    pub fn list(&self) -> &Vec<Box<dyn $dynty>> {
        &self.vlist
    }


}



    )
}


/********************* test **********************/ 
pub trait Test78756388732645 : Field + DynClone {
    fn getv(&self) -> u8 { 0 }
}
clone_trait_object!{Test78756388732645}

fn test_create_838464857639363(_a:&[u8])->Ret<(Box<dyn Test78756388732645>, usize)>{errf!("")}
combi_dynlist!{ Test294635492624,
    Uint1, Test78756388732645, test_create_838464857639363
}






