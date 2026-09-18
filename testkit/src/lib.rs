//! In-process test support for the Hacash fullnode workspace.
//!
//! The module structure deliberately follows the former `fullnodedev/testkit`,
//! while its implementations target the `base` 1.0 contracts.  It contains no
//! global protocol setup and can therefore be used safely by parallel test
//! crates that provide their own registry/profile.

pub mod sim;
