use std::any::*;

use sys::*;
use field::*;
use field::interface::*;
use protocol::*;
use protocol::interface::*;
use protocol::state::*;
use protocol::action::*;
use protocol::operate::*;
use mint::genesis::*;

use super::*;
use super::rt::*;
// use super::util::*;

// 

// pub fn try_create(_kind: u16, _buf: &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>> {
//     Ok(None)
// }



include!{"asset.rs"}
include!{"astselect.rs"}
include!{"astif.rs"}
include!{"blob.rs"}
include!{"contract.rs"}
include!{"maincall.rs"}
include!{"envfunc.rs"}




/*
    action register
*/
action_register! {
    AssetCreate          // 16
    
    AstIf                // 101
    AstSelect            // 102
    TxMessage            // 120
    TxBlob               // 121
    ContractDeploy       // 122
    ContractUpdate       // 123
    ContractMainCall     // 124

    EnvHeight            // 0x0b01 ~
    FuncHacToZhu         // 0x0f01 ~
}

