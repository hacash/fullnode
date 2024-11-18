

combi_struct!{ DiamondMintHead,
    diamond              : DiamondName    
    number               : DiamondNumber    
    prev_hash            : Hash         
    nonce                : Fixed8        
    address              : Address   
}


/*
* simple hac to
*/
action_define!{ DiamondMint, 4, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        head                 : DiamondMintHead
        custom_message       : Hash      
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}


