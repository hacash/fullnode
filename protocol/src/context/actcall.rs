use crate::action;

fn ctx_action_call(this: &mut ContextInst, k: u16, b: Vec<u8>) -> Ret<(u32, Vec<u8>)> {
    // create
    let body = vec![k.to_be_bytes().to_vec(), b].concat();
    let (action, _) = action::create(&body)?;
    action.execute(this)
}

