//! OpenCL (GPU) implementation of the X16RS mining algorithms: the Rust host
//! code lives here, and the kernels live in `opencl/*.cl` at the crate root —
//! this crate is the single home of all three implementation languages:
//! C (sphlib, `c-hash` feature), pure Rust (`pure`), and OpenCL (`ocl`
//! feature, kernels in `opencl/`).
//!
//! Only the kernel-launching and hash-level result handling lives here; action
//! building (HacdMint etc.) and mining policy (which custom message to use)
//! stay with the caller. Requires the `ocl` feature; native builds only.

pub const HASH_WIDTH: usize = super::H32S;

#[cfg(feature = "ocl")]
pub mod common;
#[cfg(feature = "ocl")]
pub mod dia;
#[cfg(feature = "ocl")]
pub mod pow;
