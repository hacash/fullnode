//! Standalone mining clients. They use the full node HTTP API and do not own
//! chain state, P2P, or server lifecycle.

pub mod diamond;
pub mod pow;
