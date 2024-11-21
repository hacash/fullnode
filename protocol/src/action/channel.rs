
/*
*
*/
action_define!{ ChannelOpen, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign
    {
        channel_id     : ChannelId
        left_bill      : AddrHac
        right_bill     : AddrHac
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}



action_define!{ ChannelClose, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign
    {
        channel_id     : ChannelId 
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}