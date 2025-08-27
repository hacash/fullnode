/*
* type 
*/





#[derive(Debug, Clone, PartialEq, PartialOrd)]
#[repr(isize)]
pub enum ActLv {
    TopOnly       =  -4isize, // only this single one on top
    TopUnique     =  -3,      // top and unique
    Top           =  -2,      // must on top
    Ast           =  -1,      // on act cond AST 
    MainCall      =   0,      // must in tx main call with depth 0
    ContractCall  =   1,      // system call or other contract call
    Any           = 127,      // any where
}

impl From<ActLv> for isize {
    fn from(n: ActLv) -> isize {
        n as isize
    }
}

impl From<&ActLv> for isize {
    fn from(n: &ActLv) -> isize {
        (*n).clone() as isize
    }
}


impl ActLv {
    
    pub fn check_depth(&self, cd: &CallDepth) -> Rerr {
        let al: isize = self.into();
        mayerr!( al < cd.0, errf!("Action level {} not support be called in depth {}", al, cd.0))
    }

}



#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub struct CallDepth(isize);

impl From<CallDepth> for isize {
    fn from(n: CallDepth) -> isize {
        n.0 as isize
    }
}

impl From<&CallDepth> for isize {
    fn from(n: &CallDepth) -> isize {
        n.0 as isize
    }
}

// impl PartialEq for CallDepth {
//     fn eq(&self, other: &Self) -> bool {
//         self.0 == other.0
//     }
// }

impl CallDepth {
    
    pub fn new(d: isize) -> Self {
        Self(d)
    }

    pub fn forward(&mut self) {
        self.0 += 1;
    }

    pub fn back(&mut self) {
        self.0 -= 1;
    }

    pub fn to_isize(&self) -> isize {
        self.0
    }

}



/*********************************/




#[derive(Default, PartialEq, Copy, Clone)]
pub enum BlkOrigin {
    #[default] Unknown, 
    Rebuild,
    Sync,
    Discover, // other find
    Mint,     // mine miner find
}


#[derive(Default, PartialEq, Copy, Clone)]
pub enum TxOrigin {
    #[default] Unknown,
    Sync,
    Broadcast, // other find
    Submit,    // mine miner find
}

