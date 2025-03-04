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
include!{"asset.rs"}
// include!{"diamond_mint.rs"}
// include!{"diamond_insc.rs"}
// include!{"diamond_util.rs"}
// include!{"channel.rs"}
include!{"chainlimit.rs"}


/*
* register
*/
action_register!{

    // hac
    HacToTrs              // 1
    HacFromTrs            // 13
    HacFromToTrs          // 14
    // HacAmountCompress     // 15
    
    // channel
    // ChannelOpen           // 2
    // ChannelClose          // 3
    
    // diamond
    // DiamondMint           // 4
    DiaSingleTrs          // 5
    DiaFromToTrs          // 6
    DiaToTrs              // 7
    DiaFromTrs            // 8
    
    // satoshi
    // SatoshiGenesis     // 9
    SatToTrs              // 10
    SatFromTrs            // 11
    SatFromToTrs          // 12

    // asset
    // AssetCreate           // 16
    AssetToTrs            // 17
    AssetFromTrs          // 18
    AssetFromToTrs        // 19

    // inscription
    // DiamondInscription         // 32
    // DiamondInscriptionClear    // 33

}
