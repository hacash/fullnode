use base::{StateLayer, StateRead, numeric_state_key, numeric_state_prefix, read_typed};
use field::{ChannelId, ChannelSto, Decode, DiamondNumber, Encode, Reader, Uint8, Uint12};
use sys::Ret;

pub struct MintState<'a>(pub &'a mut dyn StateLayer);

pub struct MintStateRead<'a>(pub &'a dyn StateRead);

const KEY_CHANNEL: u8 = numeric_state_prefix(0x0c);

/// Statistics maintained by mint consensus actions.
/// Field order is part of the state codec and must remain append-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MintTotal {
    pub minted_diamond: DiamondNumber,
    pub hacd_bid_burn_238: Uint12,
    pub opening_channel: Uint8,
    pub channel_deposit_238: Uint12,
    pub channel_deposit_sat: Uint8,
    pub channel_interest_238: Uint8,
    pub created_asset: Uint8,
    pub asset_issue_burn_238: Uint12,
    pub diamond_engraved: Uint8,
    pub diamond_insc_burn_238: Uint12,
    pub dia_insc_push: Uint8,
    pub dia_insc_clean: Uint8,
    pub dia_insc_edit: Uint8,
    pub dia_insc_move: Uint8,
    pub dia_insc_drop: Uint8,
    pub dia_insc_live_diamond: Uint8,
    pub channel_open_total: Uint8,
    pub channel_close_total: Uint8,
    pub channel_closed_hac_volume_238: Uint12,
}

impl Encode for MintTotal {
    fn size(&self) -> usize {
        self.minted_diamond.size()
            + self.hacd_bid_burn_238.size()
            + self.opening_channel.size()
            + self.channel_deposit_238.size()
            + self.channel_deposit_sat.size()
            + self.channel_interest_238.size()
            + self.created_asset.size()
            + self.asset_issue_burn_238.size()
            + self.diamond_engraved.size()
            + self.diamond_insc_burn_238.size()
            + self.dia_insc_push.size()
            + self.dia_insc_clean.size()
            + self.dia_insc_edit.size()
            + self.dia_insc_move.size()
            + self.dia_insc_drop.size()
            + self.dia_insc_live_diamond.size()
            + self.channel_open_total.size()
            + self.channel_close_total.size()
            + self.channel_closed_hac_volume_238.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.minted_diamond.encode_to(out);
        self.hacd_bid_burn_238.encode_to(out);
        self.opening_channel.encode_to(out);
        self.channel_deposit_238.encode_to(out);
        self.channel_deposit_sat.encode_to(out);
        self.channel_interest_238.encode_to(out);
        self.created_asset.encode_to(out);
        self.asset_issue_burn_238.encode_to(out);
        self.diamond_engraved.encode_to(out);
        self.diamond_insc_burn_238.encode_to(out);
        self.dia_insc_push.encode_to(out);
        self.dia_insc_clean.encode_to(out);
        self.dia_insc_edit.encode_to(out);
        self.dia_insc_move.encode_to(out);
        self.dia_insc_drop.encode_to(out);
        self.dia_insc_live_diamond.encode_to(out);
        self.channel_open_total.encode_to(out);
        self.channel_close_total.encode_to(out);
        self.channel_closed_hac_volume_238.encode_to(out);
    }
}

impl Decode for MintTotal {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut reader = Reader::new(buf);
        Ok((
            Self {
                minted_diamond: reader.read()?,
                hacd_bid_burn_238: reader.read()?,
                opening_channel: reader.read()?,
                channel_deposit_238: reader.read()?,
                channel_deposit_sat: reader.read()?,
                channel_interest_238: reader.read()?,
                created_asset: reader.read()?,
                asset_issue_burn_238: reader.read()?,
                diamond_engraved: reader.read()?,
                diamond_insc_burn_238: reader.read()?,
                dia_insc_push: reader.read()?,
                dia_insc_clean: reader.read()?,
                dia_insc_edit: reader.read()?,
                dia_insc_move: reader.read()?,
                dia_insc_drop: reader.read()?,
                dia_insc_live_diamond: reader.read()?,
                channel_open_total: reader.read()?,
                channel_close_total: reader.read()?,
                channel_closed_hac_volume_238: reader.read()?,
            },
            reader.used(),
        ))
    }
}

impl<'a> MintStateRead<'a> {
    pub fn wrap(read: &'a dyn StateRead) -> Self {
        Self(read)
    }

    pub fn channel(&self, id: &ChannelId) -> Ret<Option<ChannelSto>> {
        read_typed(self.0, &MintState::key_channel(id))
    }

    pub fn mint_total(&self) -> Ret<Option<MintTotal>> {
        read_typed(self.0, MintState::TOTAL_KEY)
    }

    pub fn get_mint_total(&self) -> Ret<MintTotal> {
        Ok(self.mint_total()?.unwrap_or_default())
    }

    pub fn hacd_bid_burn_238(&self) -> Ret<Option<Uint12>> {
        read_typed(self.0, MintState::HACD_BID_BURN_KEY)
    }

    pub fn get_hacd_bid_burn_238(&self) -> Ret<Uint12> {
        Ok(self.hacd_bid_burn_238()?.unwrap_or_default())
    }
}

impl<'a> MintState<'a> {
    pub const TOTAL_KEY: &'static [u8] = b"_mint.total";
    /// Consensus accumulator for diamond bid-fee HAC burned (238 units).
    /// `MintTotal.hacd_bid_burn_238` is a display copy of this value, not a
    /// second increment. Missing reads as 0; accumulation starts at the first
    /// burn (this node resyncs from genesis).
    pub const HACD_BID_BURN_KEY: &'static [u8] = b"_mint.hacd_bid_burn";
    pub fn wrap(layer: &'a mut dyn StateLayer) -> Self {
        Self(layer)
    }

    fn key_channel(id: &ChannelId) -> Vec<u8> {
        numeric_state_key(KEY_CHANNEL, id)
    }

    pub fn channel(&self, id: &ChannelId) -> Ret<Option<ChannelSto>> {
        read_typed(&*self.0, &Self::key_channel(id))
    }

    pub fn channel_set(&mut self, id: &ChannelId, v: &ChannelSto) {
        self.0.set(&Self::key_channel(id), v.encode());
    }

    pub fn mint_total(&self) -> Ret<Option<MintTotal>> {
        read_typed(&*self.0, Self::TOTAL_KEY)
    }

    pub fn get_mint_total(&self) -> Ret<MintTotal> {
        Ok(self.mint_total()?.unwrap_or_default())
    }

    pub fn set_mint_total(&mut self, total: &MintTotal) {
        self.0.set(Self::TOTAL_KEY, total.encode());
    }

    pub fn hacd_bid_burn_238(&self) -> Ret<Option<Uint12>> {
        read_typed(&*self.0, Self::HACD_BID_BURN_KEY)
    }

    pub fn get_hacd_bid_burn_238(&self) -> Ret<Uint12> {
        Ok(self.hacd_bid_burn_238()?.unwrap_or_default())
    }

    pub fn set_hacd_bid_burn_238(&mut self, burn: &Uint12) {
        self.0.set(Self::HACD_BID_BURN_KEY, burn.encode());
    }

    /// Add to the consensus accumulator. Zero is a no-op (key stays absent).
    /// Callers copy the returned value into `MintTotal` for display.
    #[cfg(feature = "execute")]
    pub fn add_hacd_bid_burn(&mut self, burn_238: u128) -> Ret<Uint12> {
        let mut acc = self.get_hacd_bid_burn_238()?;
        if burn_238 != 0 {
            base::total_add_u12(&mut acc, burn_238, "hacd_bid_burn")?;
            self.set_hacd_bid_burn_238(&acc);
        }
        Ok(acc)
    }

    #[allow(dead_code)]
    pub fn channel_del(&mut self, id: &ChannelId) {
        self.0.del(&Self::key_channel(id));
    }
}

pub fn with_mint_total<R>(
    state: &mut MintState,
    apply: impl FnOnce(&mut MintTotal) -> Ret<R>,
) -> Ret<R> {
    let mut total = state.get_mint_total()?;
    let result = apply(&mut total)?;
    state.set_mint_total(&total);
    Ok(result)
}
