

#[macro_export]
macro_rules! combi_dynlist {
    ($class:ident, $lenty:ty, $dynty:ident, $parseobjfunc:path) => (


#[derive(Default, Clone)]
pub struct $class {
    count: $lenty,
    vlist: Vec<Box<dyn $dynty>>
}

impl std::fmt::Debug for $class {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f,"[dyn list {}]", self.count.uint())
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
        let mut seek = self.count.parse(buf) ?;
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
        let bt1 = self.count.serialize();
        bts.push(bt1);
        for i in 0 .. *self.count as usize {
            let bt = self.vlist[i].as_ref().serialize();
            bts.push(bt);
        }
        bts.concat()
    }

    fn size(&self) -> usize {
        let mut sznum = self.count.size();
        for i in 0 .. *self.count as usize {
            sznum += self.vlist[i as usize].as_ref().size();
        }
        sznum
    }

}

impl_field_only_new!{$class}

impl $class {


    pub fn replace(&mut self, i: usize, v: Box<dyn $dynty>) -> Rerr {
        let tl = self.length() as usize;
        if i >= tl {
            return errf!("list index overflow")
        }
        self.vlist[i] = v;
        Ok(())
    }

	pub fn push(&mut self, v: Box<dyn $dynty>) -> Rerr {
        if *self.count >= <$lenty>::MAX {
            return errf!("list length overflow");
        }
		self.count += 1u8;
        self.vlist.push(v);
        Ok(())
	}

	pub fn pop(&mut self) -> Option<Box<dyn $dynty>> {
        let n = self.count.uint();
        match n {
            0 => None,
            _ => {
                self.count -= 1u8;
                self.vlist.pop()
            }
        }
	}

	pub fn length(&self) -> usize {
		*self.count as usize
	}

	pub fn count(&self) -> &$lenty {
		&self.count
	}

    pub fn list(&self) -> &Vec<Box<dyn $dynty>> {
        &self.vlist
    }


}



    )
}


/********************* test **********************/ 
pub trait Test7354846353856 : Field + DynClone {
    fn getv(&self) -> u8 { 0 }
}
clone_trait_object!{Test7354846353856}

fn test_create_823646394734(_a:&[u8])->Ret<(Box<dyn Test7354846353856>, usize)>{errf!("")}
combi_dynlist!{ Test8364856695623,
    Uint1, Test7354846353856, test_create_823646394734
}






