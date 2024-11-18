
/*
*
*/
action_define!{ SatoshiToTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        to        : AddrOrPtr
        satoshi   : Satoshi 
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}



action_define!{ SatoshiFromTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        from      : AddrOrPtr
        satoshi   : Satoshi   
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}



action_define!{ SatoshiFromToTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        from      : AddrOrPtr
        to        : AddrOrPtr
        satoshi   : Satoshi 
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}
