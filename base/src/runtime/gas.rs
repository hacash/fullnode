//! VM gas usage buckets. Hacash's transaction gas budget schedule is owned by
//! `hacash-params`; this module has only reusable runtime accounting types.

/// VM runtime gas usage, separated by resource category for metering/reporting.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct VmGasUsage {
    pub compute: i64,
    pub resource: i64,
    pub storage: i64,
}

impl VmGasUsage {
    pub fn total(&self) -> i64 {
        self.compute
            .saturating_add(self.resource)
            .saturating_add(self.storage)
    }

    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        Some(Self {
            compute: self.compute.checked_add(other.compute)?,
            resource: self.resource.checked_add(other.resource)?,
            storage: self.storage.checked_add(other.storage)?,
        })
    }

    pub fn checked_sub(self, base: Self) -> Option<Self> {
        Some(Self {
            compute: self.compute.checked_sub(base.compute)?,
            resource: self.resource.checked_sub(base.resource)?,
            storage: self.storage.checked_sub(base.storage)?,
        })
    }
}

/// Compatibility name for the VM resource-use report. This is not a Hacash
/// transaction billing meter; protocol settlement uses `TxGasMeter` instead.
pub type GasBuckets = VmGasUsage;
