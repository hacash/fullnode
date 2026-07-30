/// Fixed rules for the standard Hacash transaction and VM protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProtocolParams {
    pub ast_tree_depth_max: usize,
    pub ast_snapshot_try_gas: i64,
    pub vm: base::VmExecutionParams,
    pub diamond_form_flag: u64,
    pub max_type3_signers: usize,
    pub tex_diamond_pay_max: usize,
    pub tex_diamond_get_max_per_tx: usize,
}

pub const PROTOCOL_PARAMS: ProtocolParams = ProtocolParams {
    ast_tree_depth_max: 6,
    ast_snapshot_try_gas: 40,
    vm: base::VmExecutionParams {
        contract_store_perm_periods: 10_000,
        initial_fee_purity_floor: 50_000,
        fee_purity_reductions: &[],
    },
    diamond_form_flag: 1,
    max_type3_signers: 200,
    tex_diamond_pay_max: 60_000,
    tex_diamond_get_max_per_tx: 200,
};

pub fn execution_params(
    services: &dyn base::ExecutionServices,
) -> sys::Ret<&'static ProtocolParams> {
    services
        .execution_profile()?
        .downcast_ref::<ProtocolParams>()
        .ok_or_else(|| sys::Error::fault("standard protocol params not registered"))
}
