//! Read live VM gas/cap limits from the slotted StubVm when present.

use base::Context;

use crate::rt::{GasExtra, SpaceCap};

/// Prefer the warm `(GasExtra, SpaceCap)` from the active VM; otherwise height defaults.
///
/// Uses `vm_peek` (no take) so it is safe before/alongside `vm_call` under slot law.
pub fn peek_vm_runtime_limits(ctx: &mut dyn Context, height: u64) -> (GasExtra, SpaceCap) {
    if let Some(vm) = ctx.vm_peek() {
        if let Some(conf) = vm.runtime_config() {
            if let Ok(conf) = conf.downcast::<(GasExtra, SpaceCap)>() {
                let (gst, mut cap) = *conf;
                cap.normalize_zero_storage_period(height);
                return (gst, cap);
            }
        }
    }
    (GasExtra::new(height), SpaceCap::new(height))
}
