use std::any::Any;
use sys::*;
use field::*;
use field::interface::*;
use protocol::*;
use protocol::interface::*;
use protocol::action::*;
use protocol::operate::*;

use super::oprate::*;



include!{"channel.rs"}


/*
* actions register
*/
action_register!{

    
    // channel
    ChannelOpen           // 2
    ChannelClose          // 3


}
