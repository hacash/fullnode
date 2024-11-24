

fn cut_kind(buf: &[u8]) -> Ret<u16> {
    let mut kind = Uint2::default();
    kind.parse(buf)?;
    Ok(*kind)
}


pub fn create(buf: &[u8]) -> Ret<(Box<dyn Action>, usize)> {
    let kid = cut_kind(buf)?;
    let mut hasact = try_create(kid, buf)?;
    if let None = hasact {
        unsafe{
            hasact = EXTEND_ACTIONS_TRY_CREATE_FUNC(kid, buf)?;
        }
    }
    match hasact {
        Some(a) => Ok(a),
        None => errf!("action kind '{}' not find", kid).to_owned(),
    }
}



/*
* list defind
*/
combi_dynlist!{ DynListAction,
    Uint2, Action, create
}





