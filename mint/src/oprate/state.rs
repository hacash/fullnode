
/*
* 
*/
inst_state_define!{ MintState,

    /* status */

    1, total_count,    Empty : TotalCount
    2, latest_diamond, Empty : DiamondSmelt

    /* state */
    
    10, tx_exist,       Hash             : BlockHeight

    11, balance,        Address          : Balance
    12, channel,        ChannelId        : ChannelSto
    13, diamond,        DiamondName      : DiamondSto
    13, diamond_name ,  DiamondNumber    : DiamondName
    14, diamond_smelt,  DiamondName      : DiamondSmelt
    15, diamond_owned,  Address          : DiamondOwnedForm

}
