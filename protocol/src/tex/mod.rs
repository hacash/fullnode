use std::any::*;

use dyn_clone::*;


use field::interface::*;

use super::*;
use super::state::*;
use super::operate::*;
use super::action::*;
use super::interface::*;


static SETTLEMENT_ADDR: Address = ADDRESS_ONEX;



include!{"interface.rs"}
include!{"transfer.rs"}
include!{"condition.rs"}
include!{"check.rs"}
include!{"cell.rs"}
include!{"action.rs"}





/*
    action register
*/
action_register! {

    TexCellAct

}
