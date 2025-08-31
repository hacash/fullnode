use std::any::*;

use sys::*;
use field::*;
use field::interface::*;
use protocol::*;
use protocol::state::*;
use protocol::action::*;
use protocol::interface::*;

use super::*;
use super::rt::*;
// use super::util::*;

// 

// pub fn try_create(_kind: u16, _buf: &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>> {
//     Ok(None)
// }



include!{"astselect.rs"}
include!{"astif.rs"}
include!{"contract.rs"}
include!{"maincall.rs"}
include!{"envfunc.rs"}




/*
    action register
*/
action_register! {
    AstIf
    AstSelect
    ContractDeploy
    ContractMainCall

    EnvHeight //
    FuncHacToZhu
}

