use std::sync::*;

use sys::*;
use protocol::genesis::*;
use protocol::interface::*;
use chain::interface::*;


include!{"config.rs"}


pub mod action;


include!{"minter.rs"}

