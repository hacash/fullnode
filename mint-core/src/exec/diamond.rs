//! Diamond mint execute body.

use base::{
    BLACKHOLE_ADDR, Context, CoreState, DIAMOND_STATUS_NORMAL, diamond_owned_push_one, hacd_add,
    total_add_diamond_number, total_add_u12,
};
use field::{
    Address, Amount, BlockHeight, DiamondName, DiamondNumber, DiamondSmelt, DiamondSto,
    DiamondVisualGene, Encode, Hash, Inscripts, Uint2,
};
use sys::{Rerr, Ret, errf};
use x16rs;

use crate::action::diamond::DiamondMint;
use crate::state::{MintState, with_mint_total};

const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";

pub fn calculate_diamond_visual_gene(name: &DiamondName, life_gene: &Hash) -> DiamondVisualGene {
    let mut genehexstr = [b'0'; 20];
    let searchgx = |x| {
        for (i, a) in x16rs::DIAMOND_NAME_VALID_CHARS.iter().enumerate() {
            if *a == x {
                return HEX_CHARS[i];
            }
        }
        panic!("not supply diamond char");
    };
    for i in 0..DiamondName::SIZE {
        genehexstr[i + 2] = searchgx(name.as_ref()[i]);
    }
    let mut idx = 8;
    for i in 20..31 {
        let k = (life_gene.as_ref()[i] as usize) % 16;
        genehexstr[idx] = HEX_CHARS[k];
        idx += 1;
    }
    let mut genehex = hex::decode(genehexstr).unwrap();
    genehex[0] = life_gene.as_ref()[31];
    DiamondVisualGene::from(genehex.try_into().unwrap())
}

fn diamond_mint(this: &DiamondMint, ctx: &mut dyn Context) -> Rerr {
    let act = &this.d;
    let env = ctx.env().clone();
    let profile = ctx.services().execution_profile()?;
    let params = hacash_params::as_hacash_params(profile)
        .ok_or_else(|| sys::Error::fault("standard Hacash params not registered"))?;
    let diamond_form_flag = params.protocol.diamond_form_flag;
    let rules = params.mint_rules.diamond;
    if !env.chain.fast_sync {
        if !act.address.is_privkey() {
            return errf!("diamond mint address must be PRIVAKEY type");
        }
        check_transfer_recipient_allowed(&act.address)?;
        check_diamond_mint_tx_type(ctx, params)?;
    }
    let pending_height = env.block.height;
    let pending_hash = env.block.hash;
    let tx_bid_fee = env.tx.fee.clone();
    let dianum = act.number.uint() as u32;
    let name = act.diamond;
    let prev_hash = act.prev_hash;
    let nonce = act.nonce;
    let address = act.address;
    let custom_message = if dianum > rules.custom_message_after {
        act.custom_message.encode()
    } else {
        Vec::new()
    };
    let tx_bid_burn_238 = if dianum > rules.burn_90_percent_after {
        Some(diamond_mint_legacy_bid_burn(ctx, params, &tx_bid_fee)?.to_238_u64()? as u128)
    } else {
        None
    };

    let prev_hash_arr: &[u8; 32] = prev_hash.as_ref().try_into().unwrap();
    let nonce_arr: &[u8; 8] = nonce.as_ref().try_into().unwrap();
    let address_arr: &[u8; 21] = address.as_ref().try_into().unwrap();
    let (sha3hx, mediumhx, diahx) = x16rs::mine_diamond(
        dianum,
        prev_hash_arr,
        nonce_arr,
        address_arr,
        &custom_message,
    );

    let mut state = CoreState::wrap(ctx.layer());
    if !env.chain.fast_sync {
        if pending_hash != Hash::default() && pending_height % 5 != 0 {
            return errf!("diamond must be in a block height that is divisible by 5");
        }
        let latest = state.latest_diamond()?.unwrap_or_default();
        let latestdianum = latest.number.uint() as u32;
        let neednextnumber = latestdianum + 1;
        if dianum != neednextnumber {
            return errf!(
                "diamond number expected {} but got {}",
                neednextnumber,
                dianum
            );
        }
        if dianum > 1 && latest.born_hash != prev_hash {
            return errf!(
                "diamond prev hash expected {:?} but got {:?}",
                latest.born_hash,
                prev_hash
            );
        }
        if !x16rs::check_diamond_difficulty(dianum, &sha3hx, &mediumhx) {
            return errf!("diamond difficulty does not match");
        }
        let Some(dianame) = x16rs::check_diamond_hash_result(diahx) else {
            return errf!("diamond hash result is not a valid diamond name");
        };
        let dianame = DiamondName::from(dianame);
        if name != dianame {
            return errf!("diamond name expected {:?} but got {:?}", dianame, name);
        }
        if state.diamond(&name)?.is_some() {
            return errf!("diamond already exists");
        }
    }

    let projected_burn = MintState::wrap(&mut *state.0)
        .get_mint_total()?
        .hacd_bid_burn_238
        .uint()
        + tx_bid_burn_238.unwrap_or(0);
    let average_bid_burn = calculate_diamond_average_bid_burn(dianum, projected_burn, rules)?;
    let life_gene =
        calculate_diamond_life_gene(dianum, &mediumhx, &pending_hash, &tx_bid_fee, rules);
    let diasmelt = DiamondSmelt {
        diamond: name,
        number: act.number,
        born_height: BlockHeight::from(pending_height),
        born_hash: pending_hash,
        prev_hash,
        miner_address: address,
        bid_fee: tx_bid_fee,
        nonce,
        average_bid_burn,
        life_gene,
    };
    state.latest_diamond_set(&diasmelt);
    state.diamond_smelt_set(&name, &diasmelt);
    state.diamond_set(
        &name,
        &DiamondSto {
            status: DIAMOND_STATUS_NORMAL,
            address,
            prev_engraved_height: BlockHeight::default(),
            inscripts: Inscripts::default(),
        },
    );
    state.diamond_name_set(&act.number, &name);
    if env.chain.consensus_flags & diamond_form_flag != 0 {
        diamond_owned_push_one(&mut state, &address, &name)?;
    }
    hacd_add(&mut state, &address, &DiamondNumber::from(1))?;
    with_mint_total(&mut MintState::wrap(&mut *state.0), |ttcount| {
        total_add_diamond_number(&mut ttcount.minted_diamond, 1, "minted_diamond")?;
        if let Some(burn_238) = tx_bid_burn_238 {
            total_add_u12(
                &mut ttcount.hacd_bid_burn_238,
                burn_238,
                "hacd_bid_burn_238",
            )?;
        }
        Ok(())
    })?;
    Ok(())
}

fn check_diamond_mint_tx_type(ctx: &dyn Context, params: &hacash_params::HacashParams) -> Rerr {
    if ctx.env().tx.ty != params.protocol.tx_type_2 {
        return errf!("DiamondMint can only be executed in tx type 2");
    }
    Ok(())
}

fn diamond_mint_legacy_bid_burn(
    ctx: &dyn Context,
    params: &hacash_params::HacashParams,
    tx_bid_fee: &Amount,
) -> Ret<Amount> {
    if !ctx.env().chain.fast_sync {
        check_diamond_mint_tx_type(ctx, params)?;
    }
    tx_bid_fee.sub_mode_u128(&ctx.tx().fee_got())
}

fn check_transfer_recipient_allowed(to: &Address) -> Rerr {
    if is_privakey_unknown(to) && *to != BLACKHOLE_ADDR {
        return errf!(
            "cannot transfer to system address {:?} (privakey unknown)",
            to
        );
    }
    Ok(())
}

fn is_privakey_unknown(addr: &Address) -> bool {
    addr.version() == 0 && addr.as_ref()[..17].iter().all(|&x| x == 0)
}

fn calculate_diamond_life_gene(
    dianum: u32,
    diamhash: &[u8; 32],
    pending_block_hash: &Hash,
    diabidfee: &Amount,
    rules: hacash_params::DiamondRules,
) -> Hash {
    let mut vgenehash = *diamhash;
    if dianum > rules.visual_gene_block_hash_after {
        let mut vgenestuff = diamhash.to_vec();
        vgenestuff.extend_from_slice(pending_block_hash.as_ref());
        if dianum > rules.visual_gene_bid_fee_after {
            diabidfee.encode_to(&mut vgenestuff);
        }
        vgenehash = x16rs::calculate_hash(vgenestuff);
    }
    Hash::from(vgenehash)
}

fn calculate_diamond_average_bid_burn(
    diamond_number: u32,
    hacd_burn_238: u128,
    rules: hacash_params::DiamondRules,
) -> Ret<Uint2> {
    if diamond_number <= rules.average_bid_burn_after {
        return Ok(Uint2::from(10));
    }
    let bsnum = diamond_number - rules.burn_90_percent_after;
    let avgbid = hacd_burn_238 / 1_000_000_0000 / bsnum as u128 + 1;
    if avgbid > u16::MAX as u128 {
        return errf!("average bid burn overflow u16");
    }
    Ok(Uint2::from(avgbid as u16))
}

base::impl_action_execute! {
    DiamondMint {
        (self, ctx) {
            diamond_mint(self, ctx)?;
            Ok(vec![])
        }
    }
}
