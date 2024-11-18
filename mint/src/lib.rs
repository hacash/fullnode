use sys::*;
use protocol::interface::*;
use chain::interface::*;

include!{"config.rs"}


pub mod action;



pub struct HacashMinter {
    
}

impl HacashMinter {

    pub fn create(_: &IniObj) -> Self {
        Self {}
    }

}


impl Minter for HacashMinter {
    fn init(&self, _: &IniObj) {
        protocol::action::setup_extend_actions_try_create(empty_create);
    }
}

// 

pub fn empty_create(_kind: u16, _buf: &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>> {
    Ok(None)
}



