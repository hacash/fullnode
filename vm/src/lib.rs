
#[macro_use]
pub mod rt;
pub mod value;
pub mod space;
pub mod ir;
pub mod native;
pub mod interpreter;
pub mod frame;
pub mod machine;
pub mod action;
pub mod hook;
pub mod lang;
pub mod contract;

use machine::*;

include!{"field/mod.rs"}
include!{"interface/mod.rs"}

lazy_static::lazy_static! {
    pub static ref MACHINE_MANAGER: MachineManage = MachineManage::new();
}




