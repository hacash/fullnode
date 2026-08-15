use std::sync::Arc;

use base::{
    ActScope, ActionRef, BLACKHOLE_ADDR, Context, CoreState, DIAMOND_STATUS_NORMAL,
    diamond_owned_push_one, hacd_add, total_add_diamond_number, total_add_u12,
};
use field::{
    Address, Amount, BlockHeight, DiamondName, DiamondNumber, DiamondSmelt, DiamondSto,
    DiamondVisualGene, Encode, Fixed8, FromJSON, Hash, Inscripts, Reader, Uint2, json_decode_value,
    json_split_object,
};
use sys::{Rerr, Ret, errf};

use crate::state::{MintState, with_mint_total};

base::impl_fields_to_json!(DiamondMintData {
    diamond,
    number,
    prev_hash,
    nonce,
    address
} optional custom_message when has_custom_message);
base::impl_action_to_json!(DiamondMint { d });

pub const DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE: u32 = 20_000;
pub const DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES: u32 = 30_000;
pub const DIAMOND_ABOVE_NUMBER_OF_STATISTICS_AVERAGE_BIDDING_BURNING: u32 = 40_000;
pub const DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BLOCK_HASH: u32 = 40_000;
pub const DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BIDDING_FEE: u32 = 41_000;
pub const DIAMOND_ABOVE_NUMBER_OF_MIN_FEE_AND_FORCE_CHECK_HIGHEST: u32 = 107_000;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiamondMintData {
    pub diamond: DiamondName,
    pub number: DiamondNumber,
    pub prev_hash: Hash,
    pub nonce: Fixed8,
    pub address: Address,
    pub custom_message: Hash,
}

impl Default for DiamondMintData {
    fn default() -> Self {
        Self {
            diamond: DiamondName::default(),
            number: DiamondNumber::default(),
            prev_hash: Hash::default(),
            nonce: Fixed8::default(),
            address: Address::default(),
            custom_message: Hash::default(),
        }
    }
}

impl DiamondMintData {
    fn has_custom_message(&self) -> bool {
        self.number.uint() > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE
    }
}

impl Encode for DiamondMintData {
    fn size(&self) -> usize {
        self.diamond.size()
            + self.number.size()
            + self.prev_hash.size()
            + self.nonce.size()
            + self.address.size()
            + if self.has_custom_message() {
                self.custom_message.size()
            } else {
                0
            }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.diamond.encode_to(out);
        self.number.encode_to(out);
        self.prev_hash.encode_to(out);
        self.nonce.encode_to(out);
        self.address.encode_to(out);
        if self.has_custom_message() {
            self.custom_message.encode_to(out);
        }
    }
}

#[derive(Debug, Clone)]
pub struct DiamondMint {
    pub kind: Uint2,
    pub d: DiamondMintData,
}

impl DiamondMint {
    pub const KIND: u16 = 4;

    pub fn with(diamond: DiamondName, number: DiamondNumber) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            d: DiamondMintData {
                diamond,
                number,
                ..Default::default()
            },
        }
    }
}

impl Encode for DiamondMint {
    fn size(&self) -> usize {
        self.kind.size() + self.d.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.d.encode_to(out);
    }
}

base::impl_action! {
    DiamondMint {
        name: "diamond_mint",
        scope: ActScope::TOP_ONLY,
        min_tx_type: 2,
        extra9: |this: &DiamondMint| {
            this.d.number.uint() > DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES
        },
        req_sign: |_: &DiamondMint| vec![],
        as_transfer_like: none,
        description: |this: &DiamondMint| format!("Mint diamond <{}> number {}", this.d.diamond.to_readable(), this.d.number.uint()),
        execute: (self, ctx) {
        diamond_mint(self, ctx)?;
        Ok(vec![])
        }
    }
}

pub fn create_diamond_mint(
    _reg: &dyn base::BinaryCodecs,
    _kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind: Uint2 = r.read()?;
    if kind.uint() != DiamondMint::KIND {
        return sys::normalf!("DiamondMint codec got kind {}", kind.uint());
    }
    let diamond: DiamondName = r.read()?;
    let number: DiamondNumber = r.read()?;
    let prev_hash: Hash = r.read()?;
    let nonce: Fixed8 = r.read()?;
    let address: Address = r.read()?;
    let custom_message = if number.uint() > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        r.read()?
    } else {
        Hash::default()
    };
    Ok((
        Arc::new(DiamondMint {
            kind,
            d: DiamondMintData {
                diamond,
                number,
                prev_hash,
                nonce,
                address,
                custom_message,
            },
        }),
        r.used(),
    ))
}

fn parse_diamond_mint_json(json: &str) -> Ret<DiamondMint> {
    let mut seen = std::collections::HashSet::new();
    let mut declared_kind = Uint2::from(DiamondMint::KIND);
    let mut data_json = None;
    let mut data = DiamondMintData::default();
    let mut flat_fields = Vec::new();

    for (key, value) in json_split_object(json)? {
        if !seen.insert(key) {
            return sys::normalf!("DiamondMint JSON field {} is duplicated", key);
        }
        match key {
            "kind" => declared_kind = json_decode_value(value)?,
            "d" => data_json = Some(value),
            "diamond" | "number" | "prev_hash" | "nonce" | "address" | "custom_message" => {
                flat_fields.push((key, value));
            }
            _ => {}
        }
    }
    if declared_kind.uint() != DiamondMint::KIND {
        return sys::normalf!(
            "action kind mismatch: expected {} got {}",
            DiamondMint::KIND,
            declared_kind.uint()
        );
    }

    let fields = match data_json {
        Some(nested) => json_split_object(nested)?,
        None => flat_fields,
    };
    let mut data_seen = std::collections::HashSet::new();
    for (key, value) in fields {
        if !data_seen.insert(key) {
            return sys::normalf!("DiamondMint data field {} is duplicated", key);
        }
        match key {
            "diamond" => data.diamond.from_json(value)?,
            "number" => data.number.from_json(value)?,
            "prev_hash" => data.prev_hash.from_json(value)?,
            "nonce" => data.nonce.from_json(value)?,
            "address" => data.address.from_json(value)?,
            "custom_message" => data.custom_message.from_json(value)?,
            _ => {}
        }
    }
    if !data_seen.contains("diamond") || !data_seen.contains("number") {
        return sys::errf!("DiamondMint JSON requires diamond and number");
    }
    Ok(DiamondMint {
        kind: declared_kind,
        d: data,
    })
}

impl base::ActionJsonCodec for DiamondMint {
    fn decode_json(json: &str) -> Ret<Self> {
        parse_diamond_mint_json(json)
    }
}

impl field::FromJSON for DiamondMint {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = parse_diamond_mint_json(json)?;
        Ok(())
    }
}

/// JSON creator for the consensus-specific diamond mint payload.
///
/// The public transaction API historically accepted both the canonical
/// `{"kind":4,"d":{...}}` form and a flat object. Keep both forms here so
/// registry dispatch does not change the API contract while ordinary actions
/// use the generated decoder.
pub fn decode_diamond_mint_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    if kind != DiamondMint::KIND {
        return sys::normalf!("DiamondMint JSON codec got kind {}", kind);
    }
    Ok(Arc::new(parse_diamond_mint_json(json)?))
}

fn diamond_mint(this: &DiamondMint, ctx: &mut dyn Context) -> Rerr {
    let act = &this.d;
    let env = ctx.env().clone();
    let diamond_form_flag = protocol::execution_params(ctx.services().as_ref())?.diamond_form_flag;
    if !env.chain.fast_sync {
        if !act.address.is_privkey() {
            return errf!("diamond mint address must be PRIVAKEY type");
        }
        check_transfer_recipient_allowed(&act.address)?;
        check_diamond_mint_tx_type(ctx)?;
    }
    let pending_height = env.block.height;
    let pending_hash = env.block.hash;
    let tx_bid_fee = env.tx.fee.clone();
    let dianum = act.number.uint() as u32;
    let name = act.diamond;
    let prev_hash = act.prev_hash;
    let nonce = act.nonce;
    let address = act.address;
    let custom_message = if dianum > DIAMOND_ABOVE_NUMBER_OF_CREATE_BY_CUSTOM_MESSAGE {
        act.custom_message.encode()
    } else {
        Vec::new()
    };
    let tx_bid_burn_238 = if dianum > DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES {
        Some(diamond_mint_legacy_bid_burn(ctx, &tx_bid_fee)?.to_238_u64()? as u128)
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
    let average_bid_burn = calculate_diamond_average_bid_burn(dianum, projected_burn)?;
    let life_gene = calculate_diamond_life_gene(dianum, &mediumhx, &pending_hash, &tx_bid_fee);
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

fn check_diamond_mint_tx_type(ctx: &dyn Context) -> Rerr {
    if ctx.env().tx.ty != protocol::tx_std::TransactionType2::TYPE {
        return errf!("DiamondMint can only be executed in tx type 2");
    }
    Ok(())
}

fn diamond_mint_legacy_bid_burn(ctx: &dyn Context, tx_bid_fee: &Amount) -> Ret<Amount> {
    if !ctx.env().chain.fast_sync {
        check_diamond_mint_tx_type(ctx)?;
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
) -> Hash {
    let mut vgenehash = *diamhash;
    if dianum > DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BLOCK_HASH {
        let mut vgenestuff = diamhash.to_vec();
        vgenestuff.extend_from_slice(pending_block_hash.as_ref());
        if dianum > DIAMOND_ABOVE_NUMBER_OF_VISUAL_GENE_APPEND_BIDDING_FEE {
            diabidfee.encode_to(&mut vgenestuff);
        }
        vgenehash = x16rs::calculate_hash(vgenestuff);
    }
    Hash::from(vgenehash)
}

fn calculate_diamond_average_bid_burn(diamond_number: u32, hacd_burn_238: u128) -> Ret<Uint2> {
    if diamond_number <= DIAMOND_ABOVE_NUMBER_OF_STATISTICS_AVERAGE_BIDDING_BURNING {
        return Ok(Uint2::from(10));
    }
    let bsnum = diamond_number - DIAMOND_ABOVE_NUMBER_OF_BURNING90_PERCENT_TX_FEES;
    let avgbid = hacd_burn_238 / 1_000_000_0000 / bsnum as u128 + 1;
    if avgbid > u16::MAX as u128 {
        return errf!("average bid burn overflow u16");
    }
    Ok(Uint2::from(avgbid as u16))
}
