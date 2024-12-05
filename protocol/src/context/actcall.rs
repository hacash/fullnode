use crate::action;

fn ctx_action_call(this: &mut ContextInst, k: u16, b: Vec<u8>) -> Ret<(i64, Vec<u8>)> {
    // create
    let body = vec![k.to_be_bytes().to_vec(), b].concat();
    let (action, _) = action::create(&body)?;
    action.execute(this).map(|(a,b)|(a as i64, b))
}

