
/*
    Extend action
*/

pub type FnExtendActionsTryCreateFunc = fn(u16, &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>>;

pub static mut EXTEND_ACTIONS_TRY_CREATE_FUNC: FnExtendActionsTryCreateFunc = |t,_|errf!("action kind '{}' not find", t).to_owned();

pub fn setup_extend_actions_try_create(f: FnExtendActionsTryCreateFunc) {
    unsafe {
        EXTEND_ACTIONS_TRY_CREATE_FUNC = f;
    }
}



/*
    Action hook
*/

pub type FnActionHookFunc = fn(u16, _: &dyn Any, _: &mut dyn Context) -> Rerr ;

pub static mut ACTION_HOOK_FUNC: FnActionHookFunc = |_,_,_|Ok(());

pub fn setup_action_hook(f: FnActionHookFunc) {
    unsafe {
        ACTION_HOOK_FUNC = f;
    }
}

