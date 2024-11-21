use std::sync::*;

use sys::*;
use field::*;
use protocol::state::*;
use protocol::genesis::*;
use protocol::interface::*;
use chain::interface::*;


include!{"config.rs"}


pub mod action;


include!{"check/init.rs"}
include!{"minter.rs"}

