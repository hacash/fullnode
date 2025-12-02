
/*
    Extend action
*/



pub type FnExtendActionsTryCreateFunc = fn(u16, &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>>;

macro_rules! fneatcf {
    () => {
        |_,_|Ok(None)
    };
}
pub static mut EXTEND_ACTIONS_TRY_CREATE_FUNCS: [FnExtendActionsTryCreateFunc; 4] = [try_create, fneatcf!(), fneatcf!(), fneatcf!()];


pub fn setup_extend_actions_try_create(idx: usize, f: FnExtendActionsTryCreateFunc) {
    unsafe {
        // println!("================= EXTEND_ACTIONS_TRY_CREATE_FUNCS[idx] = f = {}", idx);
        EXTEND_ACTIONS_TRY_CREATE_FUNCS[idx] = f;
    }
}



/*
    Action hook
*/

pub type FnActionHookFunc = fn(u16, _act: &dyn Any, _ctx: &mut dyn Context, _gas: &mut u32) -> Rerr ;

pub static mut ACTION_HOOK_FUNC: FnActionHookFunc = |_,_,_,_|Ok(());

pub fn setup_action_hook(f: FnActionHookFunc) {
    unsafe {
        ACTION_HOOK_FUNC = f;
    }
}

