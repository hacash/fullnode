use std::time::SystemTime;
use std::net::SocketAddr;
use std::collections::VecDeque;
use std::sync::atomic::{ AtomicU64, Ordering };
use std::sync::{ Arc, Mutex as StdMutex };

use tokio::net::*;
use tokio::net::tcp::*;

use sys::*;
use field::*;
use field::interface::*;

use super::p2p::*;



pub const PEER_KEY_SIZE: usize = 16;
pub type PeerKey = [u8; PEER_KEY_SIZE];


include!{"trait.rs"}
include!{"know.rs"}
include!{"peer.rs"}
include!{"send.rs"}

