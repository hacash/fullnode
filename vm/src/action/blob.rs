

action_define!{TxMessage, 96, 
    ActLv::TopUnique, // level
    false, [],
    {
        data:    BytesW1
    },
    (self, ctx, gas {
        Ok(vec![])
    })
}


action_define!{TxBlob, 97, 
    ActLv::TopUnique, // level
    false, [],
    {
        data:    BytesW2
    },
    (self, ctx, gas {
        Ok(vec![])
    })
}




