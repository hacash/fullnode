use std::any::Any;
use sys::*;
use field::*;
use field::interface::*;
use protocol::*;
use protocol::interface::*;
use protocol::transaction::*;
use protocol::action::*;
use protocol::state::*;
use protocol::operate::*;

use super::oprate::*;
// use super::genesis::*;



include!{"channel.rs"}
include!{"diamond_util.rs"}
include!{"diamond_mint.rs"}
include!{"diamond_insc.rs"}
// include!{"asset.rs"}
include!{"util.rs"}


/*
* actions register
*/
action_register!{

    
    // channel
    ChannelOpen           // 2
    ChannelClose          // 3
    DiamondMint           // 4

    // asset
    // AssetCreate           // 16

    // inscription
    DiamondInscription         // 32
    DiamondInscriptionClear    // 33


}
