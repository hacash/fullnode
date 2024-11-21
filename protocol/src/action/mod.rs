use sys::*;
use field::interface::*;
use field::*;


use super::*;
use super::operate::*;
use super::state::*;


include!{"init.rs"}
include!{"macro.rs"}
include!{"create.rs"}

include!{"hacash.rs"}
include!{"satoshi.rs"}
include!{"diamond.rs"}
include!{"diamond_mint.rs"}
include!{"diamond_insc.rs"}
include!{"diamond_util.rs"}
include!{"channel.rs"}
include!{"chainlimit.rs"}


/*
* register
*/
action_register!{
    // hac
    HacToTransfer
    HacFromTransfer
    HacFromToTransfer
    // diamond
    DiamondMint
}





