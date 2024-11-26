use std::any::*;

use sys::*;
use field::interface::*;
use field::*;


use super::*;
use super::operate::*;
use super::state::*;


include!{"util.rs"}
include!{"hook.rs"}
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
    HacToTransfer              // 1
    HacFromTransfer            // 13
    HacFromToTransfer          // 14
    
    // channel
    ChannelOpen                // 2
    ChannelClose               // 3
    
    // diamond
    DiamondMint                // 4
    DiamondSingleTransfer      // 5
    DiamondFromToTransfer      // 6
    DiamondToTransfer          // 7
    DiamondFromTransfer        // 8

    DiamondInscription         // 32
    DiamondInscriptionClear    // 33
    
    // satoshi
    SatoshiToTransfer          // 9
    SatoshiFromTransfer        // 10
    SatoshiFromToTransfer      // 11

}
