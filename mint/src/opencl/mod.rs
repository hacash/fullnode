pub const HASH_WIDTH: usize = crate::diamond_mining::HASH_WIDTH;

#[cfg(feature = "ocl")]
pub mod common;
#[cfg(feature = "ocl")]
pub mod dia;
#[cfg(feature = "ocl")]
pub mod pow;
