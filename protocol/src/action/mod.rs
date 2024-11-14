use sys::*;
use field::interface::*;
use field::*;
use super::*;
use super::interface::*;


macro_rules! not_find_action_kind_error {
    ($t:expr) => {
        Err(format!("action kind '{}' not find", $t).to_owned())
    };
}


pub type FnExtendActionsTryCreateFunc= fn(u16, &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>>;
pub static mut EXTEND_ACTIONS_TRY_CREATE_FUNC: FnExtendActionsTryCreateFunc = |t,_|not_find_action_kind_error!(t);



include!{"macro.rs"}
include!{"create.rs"}



/*
* register
*/
action_register!{
    Test63856464969364
}


/*
* list defind
*/
combi_dynlist!{ DynListAction,
    Uint2, Action, create
}





