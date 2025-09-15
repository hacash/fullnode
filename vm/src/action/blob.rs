

action_define!{TxMessage, 120, 
    ActLv::TopOnly, // level
    false, [],
    {
        data:    BytesW1
    },
    (self, ctx, gas {
        Ok(vec![])
    })
}


action_define!{TxBlob, 121, 
    ActLv::TopOnly, // level
    false, [],
    {
        data:    BytesW2
    },
    (self, ctx, gas {
        Ok(vec![])
    })
}




