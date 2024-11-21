
/*
*
*/
action_define!{ DiamondInscription, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign
    {
        diamonds         : DiamondNameListMax200
        protocol_cost    : Amount
        engraved_type    : Uint1
        engraved_content : BytesW1  
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}


action_define!{ DiamondInscriptionClear, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    [], // need sign
    {
        diamonds      : DiamondNameListMax200    
        protocol_cost : Amount
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}


