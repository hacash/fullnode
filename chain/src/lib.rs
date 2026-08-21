//! The Hacash chain engine: `discover` and `sync` funnel into `insert_block`;
//! `roll` persists stable blocks. One mutex serializes insertion and root movement.

mod apply;
mod boot;
mod engine;
mod history;
mod persist;
mod pipeline;
mod ring;
mod side_list;
mod sync;
#[cfg(test)]
mod test_engine;
mod tree;
mod verify;
mod view;

pub use engine::ChainEngine;
