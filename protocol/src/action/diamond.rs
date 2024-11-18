
/*
* 
*/
action_define!{ DiamondSingleTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        diamond   : DiamondName  
        to        : AddrOrPtr 
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}


/*
* 
*/
action_define!{ DiamondToTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        to        : AddrOrPtr
        diamonds  : DiamondNameListMax200
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}


/*
* 
*/
action_define!{ DiamondFromTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        from      : AddrOrPtr
        diamonds  : DiamondNameListMax200 
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}


/*
* 
*/
action_define!{ DiamondFromToTransfer, 1, 
    ActLv::TOP_ONLY, // level
    false, // burn 90 fee
    {
        from      : AddrOrPtr
        to        : AddrOrPtr
        diamonds  : DiamondNameListMax200
    },
    (self, _ctx, _gas {
        Ok(vec![])
    })
}

