//! The Hacash chain engine.
//!
//! Two operations matter: insert one block (`discover`) and insert a stream of
//! them (`sync`). Both funnel into `insert::insert_block`, which executes the
//! block on a snapshot of its parent's state and attaches it to the fork tree.
//! When the tree grows past `unstable_block`, `roll` persists the newly stable
//! blocks in one batch and moves the root.
//!
//! Locking is deliberately blunt: one mutex serializes all insertion, and the
//! fork tree has a short-lived lock of its own. Readers take neither — they
//! receive a snapshot that owns the state layers it needs, so they can never
//! observe a torn tree nor block a writer.

mod boot;
mod engine;
mod history;
mod apply;
mod ring;
mod persist;
mod side_list;
mod source;
mod sync;
mod tree;
mod verify;
mod view;

pub use engine::ChainEngine;
pub use source::{LocalReplay, OneShot};
