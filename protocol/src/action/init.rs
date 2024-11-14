

pub type FnExtendActionsTryCreateFunc = fn(u16, &[u8]) -> Ret<Option<(Box<dyn Action>, usize)>>;

static mut EXTEND_ACTIONS_TRY_CREATE_FUNC: FnExtendActionsTryCreateFunc = |t,_|errf!("action kind '{}' not find", t).to_owned();

pub fn setup_extend_actions_try_create(f: FnExtendActionsTryCreateFunc) {
    unsafe {
        EXTEND_ACTIONS_TRY_CREATE_FUNC = f;
    }
}




