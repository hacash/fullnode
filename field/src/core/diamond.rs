
pub type DiamondName = Fixed6;
impl DiamondName {

    pub fn name(&self) -> String {
        String::from_utf8(self.serialize()).unwrap()
    }

    pub fn is_valid(stuff: &[u8]) -> bool {
        const DIAMOND_NAME_VALID_CHARS: [u8; 16] =  *b"WTYUIAHXVMEKBSZN";
        if 6 != stuff.len() {
            return false // length not match
        }
        // all 6 char is in "WTYUIAHXVMEKBSZN"
        let rrr = stuff.iter().all(|&x|
            DIAMOND_NAME_VALID_CHARS.iter().position(|&a|a==x).is_some()
        );
        // println!("DiamondName::is_valid({}) => {}", std::str::from_utf8(&stuff).unwrap(), rrr);
        rrr
    }
}




// ******** DiamondNumberOptional and Auto ********

pub type DiamondNumberAuto = Fold64;
combi_optional!{ DiamondNumberOptional, 
    diamond: DiamondNumber
}
impl DiamondNumberAuto {
	pub fn to_diamond(&self) -> DiamondNumber {
		DiamondNumber::from( self.uint() as u32 )
	}
	pub fn from_diamond(dia: &DiamondNumber) -> DiamondNumberAuto {
		DiamondNumberAuto::from( dia.uint() as u64 ).unwrap()
	}
}



macro_rules! define_diamond_name_list { ( $class: ident, $nty: ty, $max: expr ) => {

/*
* Diamond Name List
*/
combi_list!{ $class, 
	$nty, DiamondName
}


impl $class {

    pub fn one(dia: DiamondName) -> Self {        
        let mut obj = Self::default();
        obj.push(dia).unwrap();
        obj
    }

    pub fn check(&self) -> Ret<usize> {
        // check len
        let setlen = *self.count as usize;
        let reallen = self.lists.len() as usize ;
        if setlen != reallen {
            return errf!("check fail: length need {} but got {}", setlen, reallen)
        }
        if reallen == 0 {
            return errf!("diamonds quantity cannot be zero")
        }
        if reallen > $max {
            return errf!("diamonds quantity cannot over {}", $max)
        }
        // check name
        for v in &self.lists {
            if ! DiamondName::is_valid(v.as_ref()) {
                return errf!("diamond name {} is not valid", v.to_readable())
            }
        }
        // success
        Ok(reallen)
    }
    
    pub fn contains(&self, x: &[u8]) -> bool {
        for v in &self.lists {
            if x == v.as_ref() {
                return true
            }
        }
        false // not find
    }

    pub fn splitstr(&self) -> String {
        self.lists.iter().map(|a|a.to_readable()).collect::<Vec<_>>().join(",")
    }

    pub fn readable(&self) -> String {
        self.lists.iter().map(|a|a.to_readable()).collect::<Vec<_>>().concat()
    }

    pub fn form(&self) -> Vec<u8> {
        self.lists.iter().map(|a|a.serialize()).collect::<Vec<_>>().concat()
    }

    pub fn hashset(&self) -> HashSet<DiamondName> {
        self.lists.iter().map(|a|a.clone()).collect::<HashSet<_>>()
    }

    pub fn from_readable(stuff: &str) -> Ret<$class> {
        let s = stuff.replace(" ","").replace("\n","").replace("|","").replace(",","");
        if s.len() == 0 {
            return errf!("diamond list empty")
        }
        if s.len() % 6 != 0 {
            return errf!("diamond list format error")
        }
        let num = s.len() / 6;
        if num > $max  {
            return errf!("diamond list max {} overflow", $max)
        }
        let mut obj = $class::default();
        let bs = s.as_bytes();
        for i in 0 .. num {
            let x = i*6;
            let name = DiamondName::from( bufcut!(bs, x, x+6) );
            obj.push(name).unwrap();
        }
        obj.check()?;
        Ok(obj)
    }

    
    pub fn checked_append(&mut self, dias: Vec<DiamondName>) -> Rerr {
        let n = self.lists.len() + dias.len();
        if n > $max {
            return errf!("diamond list max {} overflow", $max)
        }
        self.append(dias)?;
        // check repeat
        if self.hashset().len() != self.length() {
            return errf!("diamond name list contains duplicates")
        }
        Ok(())
    }
    

}

}}



define_diamond_name_list!{ DiamondNameListMax200,   Uint1, 200 }
define_diamond_name_list!{ DiamondNameListMax60000, Uint2, 60000 }


