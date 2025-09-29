
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


use std::sync::OnceLock;
static MACHINE_MANAGER_INSTANCE: OnceLock<MachineManage> = OnceLock::new();

pub fn global_machine_manager() -> &'static MachineManage {
    MACHINE_MANAGER_INSTANCE.get_or_init(||
        MachineManage::new()
    )
}


