

macro_rules! asset_operate_define {
    ($func_name: ident, $addr:ident, $amt:ident, $oldamt:ident,  $newsatblock:block) => (

        pub fn $func_name(ctx: &mut dyn Context, $addr: &Address, $amt: &AssetAmt) -> Ret<AssetAmt> {
            if *$amt.amount == 0 {
                return errf!("Asset value cannot zore")
            }    
            let mut state = CoreState::wrap(ctx.state());
            let mut userbls = state.balance( $addr ).unwrap_or_default();
            let $oldamt = &userbls.asset_must($amt.serial);
            /* -------- */
            let newast = $newsatblock;// operate
            /* -------- */
            // save
            userbls.asset_set(newast.clone())?;
            state.balance_set($addr, &userbls);
            Ok(newast)
        }

    )
}


/**************************** */

asset_operate_define!(asset_add, addr, asset, oldasset, {
    // do add
    oldasset.checked_add(asset)?
});

asset_operate_define!(asset_sub, addr, asset, oldasset, {  
    // check
    if oldasset < asset {
		return errf!("address {} asset {} is insufficient, at least {}", 
            addr.readable(), oldasset, asset)
    }
    // do sub
    oldasset.checked_sub(asset)?
});



/**************************** */


pub fn asset_transfer(ctx: &mut dyn Context, addr_from: &Address, addr_to: &Address, asset: &AssetAmt
) -> Ret<Vec<u8>> {
    if addr_from == addr_to {
		return errf!("cannot trs to self")
    }
    // do transfer
    asset_sub(ctx, addr_from, asset)?;
    asset_add(ctx, addr_to, asset)?;
    // ok
    Ok(vec![])
}


pub fn asset_check(ctx: &mut dyn Context, addr: &Address, ast: &AssetAmt) -> Ret<AssetAmt> {
    if *ast.amount == 0 {
        return errf!("check asset is cannot empty")
    }
    let state = CoreState::wrap(ctx.state());
    if let Some(bls) = state.balance( addr ) {
        let usrasset = bls.asset_must(ast.serial);
        if usrasset >= *ast {
            return Ok(usrasset)
        }
    }
    errf!("address {} asset is insufficient, at least {}", addr.readable(), ast)
}






