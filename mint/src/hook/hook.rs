

pub fn __empty_action_hook(kid: u16, action: &dyn Any, _: &mut dyn Context) -> Rerr {

    if kid == 4 { // mint
        if let Some(_act) = action.downcast_ref::<DiaSingleTrs>() {
            // println!("DiamondMint: {}-{}", *act.d.number, act.d.diamond.to_readable());
        }
    }

    Ok(())
}