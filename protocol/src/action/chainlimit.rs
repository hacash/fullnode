
/*
*
*/
action_define!{ SubmitHeightLimit, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        start: BlockHeight
        end:   BlockHeight
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}

action_define!{ SubChainID, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        chain_id: Uint8  
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}