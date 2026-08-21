/// TEX settlement address (system address, no known private key); a protocol
/// constant kept in `params` (not gated `exec`) so non-exec codecs can read it.
#[cfg_attr(not(feature = "execute"), allow(dead_code))]
pub const SETTLEMENT_ADDR: field::Address = field::Address::from([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]);

#[cfg(feature = "execute")]
pub fn execution_params(
    services: &dyn base::ExecutionServices,
) -> sys::Ret<&'static hacash_params::ProtocolParams> {
    let profile = services.execution_profile()?;
    hacash_params::as_hacash_params(profile)
        .map(|params| &params.protocol)
        .ok_or_else(|| sys::Error::fault("standard Hacash params not registered"))
}
