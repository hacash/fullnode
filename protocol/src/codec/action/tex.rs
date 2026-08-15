//! TexCellAct (kind 22) and TEX cell codecs.

use std::collections::HashMap;
use std::sync::Arc;

use base::{
    ActScope, ActionRef, Context, CoreState, ExecFrom, asset_add, asset_sub, diamond_owned_move,
    hac_add, hac_sub, hacd_move_one_diamond, hacd_transfer, sat_add, sat_sub,
};
use field::{
    Address, Amount, AssetAmt, BlockHeight, DiamondNameListMax200, DiamondNumber, Encode, Fold64,
    Hash, Reader, Satoshi, Sign, Uint1, Uint2, Uint4, json_decode_value, json_split_object,
};
use sys::{Account, Rerr, Ret, errf};

use crate::exec::tex::SETTLEMENT_ADDR;

fn tex_check_settlement_addr_privakey() -> Rerr {
    if !SETTLEMENT_ADDR.is_privkey() {
        return errf!(
            "tex settlement address {} must be PRIVAKEY type",
            SETTLEMENT_ADDR.to_readable()
        );
    }
    if !SETTLEMENT_ADDR.is_privkey_unknown() {
        return errf!(
            "tex settlement address {} must be a system address (value < u32::MAX)",
            SETTLEMENT_ADDR.to_readable()
        );
    }
    Ok(())
}

fn tex_check_asset_serial(ctx: &mut dyn Context, serial: Fold64) -> Rerr {
    if serial.is_zero() {
        return errf!("tex asset serial cannot be zero");
    }
    {
        let tex = ctx.tex_ledger();
        if tex.asset_is_checked(serial) {
            return Ok(());
        }
    }
    let exist = {
        let state = CoreState::wrap(ctx.layer());
        state.asset(&serial)?.is_some()
    };
    if !exist {
        return errf!("tex asset <{}> does not exist", serial.uint());
    }
    ctx.tex_ledger_mut_top()?.mark_asset_checked(serial);
    Ok(())
}

fn do_diamonds_transfer(
    ctx: &mut dyn Context,
    diamonds: &DiamondNameListMax200,
    from: &Address,
    to: &Address,
) -> Ret<Vec<u8>> {
    let dianum = diamonds.check()?;
    let diamond_form_flag = crate::execution_params(ctx.services().as_ref())?.diamond_form_flag;
    let diamond_form = ctx.env().chain.consensus_flags & diamond_form_flag != 0;
    let mut state = CoreState::wrap(ctx.layer());
    for name in diamonds.as_list() {
        hacd_move_one_diamond(&mut state, from, to, name)?;
    }
    if diamond_form {
        diamond_owned_move(&mut state, from, to, diamonds)?;
    }
    hacd_transfer(
        &mut state,
        from,
        to,
        &DiamondNumber::from(dianum as u32),
        diamonds,
    )
}

fn verify_signature(hash: &Hash, addr: &Address, sign: &Sign) -> bool {
    let got = Address::from(Account::get_address_by_public_key(sign.publickey));
    got == *addr && Account::verify_signature(&hash.0, &sign.publickey, &sign.signature)
}

#[derive(Debug, Clone)]
enum TexCell {
    ZhuPay { haczhu: Fold64 },
    ZhuGet { haczhu: Fold64 },
    SatPay { satnum: Fold64 },
    SatGet { satnum: Fold64 },
    DiaPay { diamonds: DiamondNameListMax200 },
    DiaGet { dianum: DiamondNumber },
    AssetPay { asset: AssetAmt },
    AssetGet { asset: AssetAmt },
    CondZhuAtMost { haczhu: Fold64 },
    CondZhuAtLeast { haczhu: Fold64 },
    CondZhuEq { haczhu: Fold64 },
    CondSatAtMost { satoshi: Fold64 },
    CondSatAtLeast { satoshi: Fold64 },
    CondSatEq { satoshi: Fold64 },
    CondDiaAtMost { diamond: Fold64 },
    CondDiaAtLeast { diamond: Fold64 },
    CondDiaEq { diamond: Fold64 },
    CondAssetAtMost { asset: AssetAmt },
    CondAssetAtLeast { asset: AssetAmt },
    CondAssetEq { asset: AssetAmt },
    CondHeightAtMost { height: BlockHeight },
    CondHeightAtLeast { height: BlockHeight },
    CondChainIdEq { chainid: Uint4 },
}

impl TexCell {
    fn cid(&self) -> u8 {
        match self {
            Self::ZhuPay { .. } => 1,
            Self::ZhuGet { .. } => 2,
            Self::SatPay { .. } => 3,
            Self::SatGet { .. } => 4,
            Self::DiaPay { .. } => 5,
            Self::DiaGet { .. } => 6,
            Self::AssetPay { .. } => 7,
            Self::AssetGet { .. } => 8,
            Self::CondZhuAtMost { .. } => 11,
            Self::CondZhuAtLeast { .. } => 12,
            Self::CondZhuEq { .. } => 13,
            Self::CondSatAtMost { .. } => 14,
            Self::CondSatAtLeast { .. } => 15,
            Self::CondSatEq { .. } => 16,
            Self::CondDiaAtMost { .. } => 17,
            Self::CondDiaAtLeast { .. } => 18,
            Self::CondDiaEq { .. } => 19,
            Self::CondAssetAtMost { .. } => 20,
            Self::CondAssetAtLeast { .. } => 21,
            Self::CondAssetEq { .. } => 22,
            Self::CondHeightAtMost { .. } => 23,
            Self::CondHeightAtLeast { .. } => 24,
            Self::CondChainIdEq { .. } => 25,
        }
    }

    fn is_asset_transfer(&self) -> bool {
        matches!(self, Self::AssetPay { .. } | Self::AssetGet { .. })
    }

    fn size(&self) -> usize {
        Uint1::SIZE
            + match self {
                Self::ZhuPay { haczhu } | Self::ZhuGet { haczhu } => haczhu.size(),
                Self::SatPay { satnum } | Self::SatGet { satnum } => satnum.size(),
                Self::DiaPay { diamonds } => diamonds.size(),
                Self::DiaGet { dianum } => dianum.size(),
                Self::AssetPay { asset }
                | Self::AssetGet { asset }
                | Self::CondAssetAtMost { asset }
                | Self::CondAssetAtLeast { asset }
                | Self::CondAssetEq { asset } => asset.size(),
                Self::CondZhuAtMost { haczhu }
                | Self::CondZhuAtLeast { haczhu }
                | Self::CondZhuEq { haczhu } => haczhu.size(),
                Self::CondSatAtMost { satoshi }
                | Self::CondSatAtLeast { satoshi }
                | Self::CondSatEq { satoshi } => satoshi.size(),
                Self::CondDiaAtMost { diamond }
                | Self::CondDiaAtLeast { diamond }
                | Self::CondDiaEq { diamond } => diamond.size(),
                Self::CondHeightAtMost { height } | Self::CondHeightAtLeast { height } => {
                    height.size()
                }
                Self::CondChainIdEq { chainid } => chainid.size(),
            }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        Uint1::from(self.cid()).encode_to(out);
        match self {
            Self::ZhuPay { haczhu } | Self::ZhuGet { haczhu } => haczhu.encode_to(out),
            Self::SatPay { satnum } | Self::SatGet { satnum } => satnum.encode_to(out),
            Self::DiaPay { diamonds } => diamonds.encode_to(out),
            Self::DiaGet { dianum } => dianum.encode_to(out),
            Self::AssetPay { asset }
            | Self::AssetGet { asset }
            | Self::CondAssetAtMost { asset }
            | Self::CondAssetAtLeast { asset }
            | Self::CondAssetEq { asset } => asset.encode_to(out),
            Self::CondZhuAtMost { haczhu }
            | Self::CondZhuAtLeast { haczhu }
            | Self::CondZhuEq { haczhu } => haczhu.encode_to(out),
            Self::CondSatAtMost { satoshi }
            | Self::CondSatAtLeast { satoshi }
            | Self::CondSatEq { satoshi } => satoshi.encode_to(out),
            Self::CondDiaAtMost { diamond }
            | Self::CondDiaAtLeast { diamond }
            | Self::CondDiaEq { diamond } => diamond.encode_to(out),
            Self::CondHeightAtMost { height } | Self::CondHeightAtLeast { height } => {
                height.encode_to(out)
            }
            Self::CondChainIdEq { chainid } => chainid.encode_to(out),
        }
    }

    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let cid: Uint1 = r.read()?;
        let cell = match cid.uint() {
            1 => Self::ZhuPay { haczhu: r.read()? },
            2 => Self::ZhuGet { haczhu: r.read()? },
            3 => Self::SatPay { satnum: r.read()? },
            4 => Self::SatGet { satnum: r.read()? },
            5 => Self::DiaPay {
                diamonds: r.read()?,
            },
            6 => Self::DiaGet { dianum: r.read()? },
            7 => Self::AssetPay { asset: r.read()? },
            8 => Self::AssetGet { asset: r.read()? },
            11 => Self::CondZhuAtMost { haczhu: r.read()? },
            12 => Self::CondZhuAtLeast { haczhu: r.read()? },
            13 => Self::CondZhuEq { haczhu: r.read()? },
            14 => Self::CondSatAtMost { satoshi: r.read()? },
            15 => Self::CondSatAtLeast { satoshi: r.read()? },
            16 => Self::CondSatEq { satoshi: r.read()? },
            17 => Self::CondDiaAtMost { diamond: r.read()? },
            18 => Self::CondDiaAtLeast { diamond: r.read()? },
            19 => Self::CondDiaEq { diamond: r.read()? },
            20 => Self::CondAssetAtMost { asset: r.read()? },
            21 => Self::CondAssetAtLeast { asset: r.read()? },
            22 => Self::CondAssetEq { asset: r.read()? },
            23 => Self::CondHeightAtMost { height: r.read()? },
            24 => Self::CondHeightAtLeast { height: r.read()? },
            25 => Self::CondChainIdEq { chainid: r.read()? },
            i => return errf!("cannot find tex cell id '{}'", i),
        };
        Ok((cell, r.used()))
    }

    fn execute(&self, ctx: &mut dyn Context, taradr: &Address) -> Rerr {
        match self {
            Self::ZhuPay { haczhu } => {
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_sub(ctx, taradr, &amt)?;
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(zhures) = tex.zhu.checked_add(zhu as i64) else {
                    return errf!("cell state coin zhu overflow");
                };
                tex.zhu = zhures;
                Ok(())
            }
            Self::ZhuGet { haczhu } => {
                let zhu = haczhu.uint();
                if zhu > 10_000_000_000_000_000 {
                    return errf!("cell zhu too large");
                }
                let amt = Amount::zhu(zhu);
                hac_add(ctx, taradr, &amt)?;
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(zhures) = tex.zhu.checked_sub(zhu as i64) else {
                    return errf!("cell state coin zhu overflow");
                };
                tex.zhu = zhures;
                Ok(())
            }
            Self::SatPay { satnum } => {
                let sat = Satoshi::from(satnum.uint());
                sat_sub(ctx, taradr, &sat)?;
                let n = satnum.uint();
                if n > i64::MAX as u64 {
                    return errf!("cell sat too large");
                }
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(satres) = tex.sat.checked_add(n as i64) else {
                    return errf!("cell state coin sat overflow");
                };
                tex.sat = satres;
                Ok(())
            }
            Self::SatGet { satnum } => {
                let sat = Satoshi::from(satnum.uint());
                sat_add(ctx, taradr, &sat)?;
                let n = satnum.uint();
                if n > i64::MAX as u64 {
                    return errf!("cell sat too large");
                }
                let tex = ctx.tex_ledger_mut_top()?;
                let Some(satres) = tex.sat.checked_sub(n as i64) else {
                    return errf!("cell state coin sat overflow");
                };
                tex.sat = satres;
                Ok(())
            }
            Self::DiaPay { diamonds } => {
                tex_check_settlement_addr_privakey()?;
                diamonds.check()?;
                do_diamonds_transfer(ctx, diamonds, taradr, &SETTLEMENT_ADDR)?;
                let max = crate::execution_params(ctx.services().as_ref())?.tex_diamond_pay_max;
                ctx.tex_ledger_mut_top()?.record_diamond_pay(diamonds, max)
            }
            Self::DiaGet { dianum } => {
                if dianum.uint() == 0 {
                    return errf!("cell diamond get: number cannot be zero");
                }
                let max =
                    crate::execution_params(ctx.services().as_ref())?.tex_diamond_get_max_per_tx;
                ctx.tex_ledger_mut_top()?
                    .record_diamond_get(*taradr, dianum.uint() as usize, max)
            }
            Self::AssetPay { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                {
                    let mut state = CoreState::wrap(ctx.layer());
                    asset_sub(&mut state, taradr, asset)?;
                }
                ctx.tex_ledger_mut_top()?.record_asset_pay(asset)?;
                Ok(())
            }
            Self::AssetGet { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                {
                    let mut state = CoreState::wrap(ctx.layer());
                    asset_add(&mut state, taradr, asset)?;
                }
                ctx.tex_ledger_mut_top()?.record_asset_get(*taradr, asset)?;
                Ok(())
            }
            Self::CondZhuAtMost { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu >= bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondZhuAtLeast { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu <= bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondZhuEq { haczhu } => {
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let zhu = Amount::zhu(haczhu.uint());
                if zhu == bls.hacash {
                    Ok(())
                } else {
                    errf!("cell condition zhu check failed")
                }
            }
            Self::CondSatAtMost { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() >= sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondSatAtLeast { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() <= sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondSatEq { satoshi } => {
                let sat = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .satoshi
                    .uint();
                if satoshi.uint() == sat {
                    Ok(())
                } else {
                    errf!("cell condition sat check failed")
                }
            }
            Self::CondDiaAtMost { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() >= dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondDiaAtLeast { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() <= dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondDiaEq { diamond } => {
                let dia = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default()
                    .diamond
                    .uint();
                if diamond.uint() == dia {
                    Ok(())
                } else {
                    errf!("cell condition dia check failed")
                }
            }
            Self::CondAssetAtMost { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() >= ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondAssetAtLeast { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() <= ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondAssetEq { asset } => {
                tex_check_asset_serial(ctx, asset.serial)?;
                let bls = CoreState::wrap(ctx.layer())
                    .balance(taradr)?
                    .unwrap_or_default();
                let ast = bls.asset_must(asset.serial)?;
                if asset.amount.uint() == ast.amount.uint() {
                    Ok(())
                } else {
                    errf!(
                        "cell condition asset <{}> check failed",
                        asset.serial.uint()
                    )
                }
            }
            Self::CondHeightAtMost { height } => {
                if height.uint() >= ctx.env().block.height {
                    Ok(())
                } else {
                    errf!("cell condition check failed")
                }
            }
            Self::CondHeightAtLeast { height } => {
                if height.uint() <= ctx.env().block.height {
                    Ok(())
                } else {
                    errf!("cell condition check failed")
                }
            }
            Self::CondChainIdEq { chainid } => {
                if ctx.env().chain.id.get() == chainid.uint() {
                    Ok(())
                } else {
                    errf!("cell condition chain id check failed")
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct TexCellAct {
    pub kind: Uint2,
    pub addr: Address,
    cells: Vec<TexCell>,
    pub sign: Sign,
}

impl field::ToJSON for TexCell {
    fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
        macro_rules! cell {
            ($id:expr, $name:literal, $value:expr) => {
                format!(
                    "{{\"cellid\":{},\"{}\":{}}}",
                    $id,
                    $name,
                    field::ToJSON::to_json_fmt($value, fmt)
                )
            };
        }

        match self {
            Self::ZhuPay { haczhu } => cell!(1, "haczhu", haczhu),
            Self::ZhuGet { haczhu } => cell!(2, "haczhu", haczhu),
            Self::SatPay { satnum } => cell!(3, "satnum", satnum),
            Self::SatGet { satnum } => cell!(4, "satnum", satnum),
            Self::DiaPay { diamonds } => cell!(5, "diamonds", diamonds),
            Self::DiaGet { dianum } => cell!(6, "dianum", dianum),
            Self::AssetPay { asset } => cell!(7, "asset", asset),
            Self::AssetGet { asset } => cell!(8, "asset", asset),
            Self::CondZhuAtMost { haczhu } => cell!(11, "haczhu", haczhu),
            Self::CondZhuAtLeast { haczhu } => cell!(12, "haczhu", haczhu),
            Self::CondZhuEq { haczhu } => cell!(13, "haczhu", haczhu),
            Self::CondSatAtMost { satoshi } => cell!(14, "satoshi", satoshi),
            Self::CondSatAtLeast { satoshi } => cell!(15, "satoshi", satoshi),
            Self::CondSatEq { satoshi } => cell!(16, "satoshi", satoshi),
            Self::CondDiaAtMost { diamond } => cell!(17, "diamond", diamond),
            Self::CondDiaAtLeast { diamond } => cell!(18, "diamond", diamond),
            Self::CondDiaEq { diamond } => cell!(19, "diamond", diamond),
            Self::CondAssetAtMost { asset } => cell!(20, "asset", asset),
            Self::CondAssetAtLeast { asset } => cell!(21, "asset", asset),
            Self::CondAssetEq { asset } => cell!(22, "asset", asset),
            Self::CondHeightAtMost { height } => cell!(23, "height", height),
            Self::CondHeightAtLeast { height } => cell!(24, "height", height),
            Self::CondChainIdEq { chainid } => cell!(25, "chainid", chainid),
        }
    }
}

fn decode_tex_cell_json(json: &str) -> Ret<TexCell> {
    let mut fields = HashMap::new();
    for (key, value) in json_split_object(json)? {
        if fields.insert(key, value).is_some() {
            return sys::decodef!("TEX cell JSON field {} is duplicated", key);
        }
    }
    let cid: Uint1 = fields
        .get("cellid")
        .copied()
        .ok_or_else(|| sys::Error::decode("TEX cell JSON missing cellid"))
        .and_then(json_decode_value)?;
    let cid = cid.uint();

    macro_rules! decode_cell {
        ($id:expr, $name:literal, $variant:ident, $field:ident) => {
            if cid == $id {
                let value = fields.get($name).copied().ok_or_else(|| {
                    sys::Error::decode(format!("TEX cell {} JSON missing {}", $id, $name))
                })?;
                return Ok(TexCell::$variant {
                    $field: json_decode_value(value)?,
                });
            }
        };
    }

    decode_cell!(1, "haczhu", ZhuPay, haczhu);
    decode_cell!(2, "haczhu", ZhuGet, haczhu);
    decode_cell!(3, "satnum", SatPay, satnum);
    decode_cell!(4, "satnum", SatGet, satnum);
    decode_cell!(5, "diamonds", DiaPay, diamonds);
    decode_cell!(6, "dianum", DiaGet, dianum);
    decode_cell!(7, "asset", AssetPay, asset);
    decode_cell!(8, "asset", AssetGet, asset);
    decode_cell!(11, "haczhu", CondZhuAtMost, haczhu);
    decode_cell!(12, "haczhu", CondZhuAtLeast, haczhu);
    decode_cell!(13, "haczhu", CondZhuEq, haczhu);
    decode_cell!(14, "satoshi", CondSatAtMost, satoshi);
    decode_cell!(15, "satoshi", CondSatAtLeast, satoshi);
    decode_cell!(16, "satoshi", CondSatEq, satoshi);
    decode_cell!(17, "diamond", CondDiaAtMost, diamond);
    decode_cell!(18, "diamond", CondDiaAtLeast, diamond);
    decode_cell!(19, "diamond", CondDiaEq, diamond);
    decode_cell!(20, "asset", CondAssetAtMost, asset);
    decode_cell!(21, "asset", CondAssetAtLeast, asset);
    decode_cell!(22, "asset", CondAssetEq, asset);
    decode_cell!(23, "height", CondHeightAtMost, height);
    decode_cell!(24, "height", CondHeightAtLeast, height);
    decode_cell!(25, "chainid", CondChainIdEq, chainid);
    Err(sys::Error::decode(format!(
        "cannot find tex cell id '{}'",
        cid
    )))
}

impl field::ToJSON for TexCellAct {
    fn to_json_fmt(&self, fmt: &field::JSONFormater) -> String {
        let cells = self
            .cells
            .iter()
            .map(|cell| cell.to_json_fmt(fmt))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"kind\":{},\"addr\":{},\"cells\":[{}],\"sign\":{}}}",
            field::ToJSON::to_json_fmt(&self.kind, fmt),
            field::ToJSON::to_json_fmt(&self.addr, fmt),
            cells,
            field::ToJSON::to_json_fmt(&self.sign, fmt)
        )
    }
}

impl TexCellAct {
    pub const KIND: u16 = 22;

    fn has_asset_transfer_cell(&self) -> bool {
        self.cells.iter().any(|c| c.is_asset_transfer())
    }

    fn cells_size(&self) -> usize {
        Uint1::SIZE + self.cells.iter().map(|c| c.size()).sum::<usize>()
    }

    fn encode_cells(&self, out: &mut Vec<u8>) {
        Uint1::from(self.cells.len() as u8).encode_to(out);
        for cell in &self.cells {
            cell.encode_to(out);
        }
    }

    fn get_sign_stuff(&self) -> Hash {
        let mut stf = Vec::with_capacity(self.addr.size() + self.cells_size());
        self.addr.encode_to(&mut stf);
        self.encode_cells(&mut stf);
        Hash::from(sys::calculate_hash(stf))
    }
}

impl Encode for TexCellAct {
    fn size(&self) -> usize {
        self.kind.size() + self.addr.size() + self.cells_size() + self.sign.size()
    }
    fn encode_to(&self, out: &mut Vec<u8>) {
        self.kind.encode_to(out);
        self.addr.encode_to(out);
        self.encode_cells(out);
        self.sign.encode_to(out);
    }
}

base::impl_action! {
    TexCellAct {
        name: "tex_cell_act",
        scope: ActScope::TOP,
        min_tx_type: 3,
        extra9: |this: &TexCellAct| this.has_asset_transfer_cell(),
        req_sign: |_: &TexCellAct| vec![],
        as_transfer_like: none,
        description: |this: &TexCellAct| {
            format!("Execute {} tex cells by {}", this.cells.len(), this.addr.to_readable())
        },
        execute: (self, ctx) {
        if ctx.exec_from() != ExecFrom::Top {
            return errf!(
                "TexCellAct can only run in TOP context, got {}",
                ctx.exec_from()
            );
        }
        self.addr.must_privkey()?;
        let thx = self.get_sign_stuff();
        if !verify_signature(&thx, &self.addr, &self.sign) {
            return errf!(
                "address {} signature verification failed in tex cell action",
                self.addr.to_readable()
            );
        }
        for cell in &self.cells {
            cell.execute(ctx, &self.addr)?;
        }
        Ok(vec![])
        }
    }
}

pub fn create_tex_cell_act(
    _reg: &dyn base::BinaryCodecs,
    kind: u16,
    buf: &[u8],
) -> Ret<(ActionRef, usize)> {
    let mut r = Reader::new(buf);
    let kind_field: Uint2 = r.read()?;
    if kind_field.uint() != kind {
        return sys::decodef!(
            "action kind mismatch: expected {} got {}",
            kind,
            kind_field.uint()
        );
    }
    let addr: Address = r.read()?;
    let count: Uint1 = r.read()?;
    let mut cells = Vec::with_capacity(count.uint() as usize);
    for _ in 0..count.uint() {
        let (cell, used) = TexCell::decode(&buf[r.used()..])?;
        let _ = r.read_bytes(used)?;
        cells.push(cell);
    }
    let sign: Sign = r.read()?;
    Ok((
        Arc::new(TexCellAct {
            kind: kind_field,
            addr,
            cells,
            sign,
        }),
        r.used(),
    ))
}

pub fn decode_tex_cell_act_json(
    _reg: &dyn base::CodecRegistry,
    kind: u16,
    json: &str,
) -> Ret<ActionRef> {
    if kind != TexCellAct::KIND {
        return sys::decodef!("TexCellAct JSON codec got kind {}", kind);
    }
    let mut fields = HashMap::new();
    for (key, value) in json_split_object(json)? {
        if fields.insert(key, value).is_some() {
            return sys::decodef!("TexCellAct JSON field {} is duplicated", key);
        }
    }
    let kind_field: Uint2 = fields
        .get("kind")
        .copied()
        .map(json_decode_value)
        .transpose()?
        .unwrap_or_else(|| Uint2::from(TexCellAct::KIND));
    if kind_field.uint() != TexCellAct::KIND {
        return sys::decodef!(
            "action kind mismatch: expected {} got {}",
            TexCellAct::KIND,
            kind_field.uint()
        );
    }
    let addr: Address = fields
        .get("addr")
        .copied()
        .ok_or_else(|| sys::Error::decode("TexCellAct JSON missing addr"))
        .and_then(json_decode_value)?;
    let cells_json = fields
        .get("cells")
        .copied()
        .ok_or_else(|| sys::Error::decode("TexCellAct JSON missing cells"))?;
    let cells = field::json_split_array(cells_json)?
        .into_iter()
        .map(decode_tex_cell_json)
        .collect::<Ret<Vec<_>>>()?;
    Uint1::from_usize(cells.len())?;
    let sign: Sign = fields
        .get("sign")
        .copied()
        .ok_or_else(|| sys::Error::decode("TexCellAct JSON missing sign"))
        .and_then(json_decode_value)?;
    Ok(Arc::new(TexCellAct {
        kind: kind_field,
        addr,
        cells,
        sign,
    }))
}
