


/*
* TotalCount
*/
combi_struct!(TotalCount, 
	minted_diamond           : DiamondNumber
	hacd_bid_burn_zhu        : Uint8 // HAC zhu
	// channel
	opening_channel          : Uint5
	channel_deposit_zhu      : Uint8 // HAC zhu
	channel_deposit_sat      : Uint8 // BTC sat
	channel_interest_zhu     : Uint8 // HAC zhu
	// diamond
	diamond_engraved         : Uint8
	diamond_insc_burn_zhu    : Uint8 // HAC zhu
	asset_issue_burn_mei     : Uint5 // HAC mei
);

