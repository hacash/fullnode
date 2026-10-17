//! Built-in process apps. [`register`] is the composition root; [`crate::fullnode::Fullnode`]
//! only schedules the resulting list.

mod bider;
mod upgrade_guard;

use sys::Rerr;

use crate::fullnode::Fullnode;

pub(crate) fn register(node: &mut Fullnode) -> Rerr {
    bider::register(node)?;
    upgrade_guard::register(node)?;
    Ok(())
}
