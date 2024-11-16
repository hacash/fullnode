
pub const P2P_HAND_SHAKE_MAGIC_NUMBER: u32 = 3418609257;
pub const P2P_MSG_DATA_MAX_SIZE: u32 = 1012 * 1024 * 32; //  32MB

// msg types

pub const MSG_REPORT_PEER: u8 = 1; // Report my port + PeerKey + peername, and request that you want to connect as a persistent node
pub const MSG_ANSWER_PEER: u8 = 2; // Reply to my PeerKey + peername and agree to use it as a persistent connection
pub const MSG_PING: u8 = 3;
pub const MSG_PONG: u8 = 4;
// Message to disconnect immediately after reply
pub const MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK: u8 = 201;
pub const MSG_REQUEST_NEAREST_PUBLIC_NODES: u8 = 202;
// Message without reply
pub const MSG_REMIND_ME_IS_PUBLIC: u8 = 151;
// Customer upper level message
pub const MSG_CLOSE: u8 = 254; // do close
pub const MSG_CUSTOMER: u8 = 255;


