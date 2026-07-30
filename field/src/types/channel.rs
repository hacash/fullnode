use sys::{Ret, errf};

use crate::codec::{Decode, Encode, Reader};
use crate::types::balance::{AddrBalance, Balance, HacSat};
use crate::types::bool::Bool;
use crate::types::fixed::Fixed16;
use crate::types::uint::{BlockHeight, Uint1, Uint2, Uint4, Uint8};

pub type ChannelId = Fixed16;

codec_struct!(ChallengePeriodData {
    is_have_challenge_log: Bool,
    challenge_launch_height: BlockHeight,
    assert_bill_auto_number: Uint8,
    assert_address_is_left_or_right: Bool,
    assert_bill: HacSat,
});

codec_struct!(ClosedDistributionData {
    left_bill: Balance,
    right_bill: Balance,
});

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChallengePeriodDataOptional {
    pub exist: Bool,
    pub challenge: Option<ChallengePeriodData>,
}

impl ChallengePeriodDataOptional {
    pub fn must(v: ChallengePeriodData) -> Self {
        Self {
            exist: Bool::new(true),
            challenge: Some(v),
        }
    }

    pub fn is_exist(&self) -> bool {
        self.exist.is_true()
    }

    pub fn value(&self) -> ChallengePeriodData {
        self.challenge.clone().unwrap_or_default()
    }
}

impl Encode for ChallengePeriodDataOptional {
    fn size(&self) -> usize {
        self.exist.size()
            + if self.is_exist() {
                self.challenge.as_ref().unwrap().size()
            } else {
                0
            }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.exist.encode_to(out);
        if self.is_exist() {
            self.challenge.as_ref().unwrap().encode_to(out);
        }
    }
}

impl Decode for ChallengePeriodDataOptional {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let exist: Bool = r.read()?;
        let challenge = if exist.is_true() {
            Some(r.read()?)
        } else {
            None
        };
        Ok((Self { exist, challenge }, r.used()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClosedDistributionDataOptional {
    pub exist: Bool,
    pub closed_distribution: Option<ClosedDistributionData>,
}

impl ClosedDistributionDataOptional {
    pub fn must(v: ClosedDistributionData) -> Self {
        Self {
            exist: Bool::new(true),
            closed_distribution: Some(v),
        }
    }

    pub fn is_exist(&self) -> bool {
        self.exist.is_true()
    }

    pub fn value(&self) -> ClosedDistributionData {
        self.closed_distribution.clone().unwrap_or_default()
    }
}

impl Encode for ClosedDistributionDataOptional {
    fn size(&self) -> usize {
        self.exist.size()
            + if self.is_exist() {
                self.closed_distribution.as_ref().unwrap().size()
            } else {
                0
            }
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.exist.encode_to(out);
        if self.is_exist() {
            self.closed_distribution.as_ref().unwrap().encode_to(out);
        }
    }
}

impl Decode for ClosedDistributionDataOptional {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let exist: Bool = r.read()?;
        let closed_distribution = if exist.is_true() {
            Some(r.read()?)
        } else {
            None
        };
        Ok((
            Self {
                exist,
                closed_distribution,
            },
            r.used(),
        ))
    }
}

pub const CHANNEL_STATUS_OPENING: Uint1 = Uint1::from(0);
pub const CHANNEL_STATUS_CHALLENGING: Uint1 = Uint1::from(1);
pub const CHANNEL_STATUS_AGREEMENT_CLOSED: Uint1 = Uint1::from(2);
pub const CHANNEL_STATUS_FINAL_ARBITRATION_CLOSED: Uint1 = Uint1::from(3);

pub const CHANNEL_INTEREST_ATTRIBUTION_TYPE_DEFAULT: Uint1 = Uint1::from(0);
pub const CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_LEFT: Uint1 = Uint1::from(1);
pub const CHANNEL_INTEREST_ATTRIBUTION_TYPE_ALL_TO_RIGHT: Uint1 = Uint1::from(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChannelSto {
    pub status: Uint1,
    pub reuse_version: Uint4,
    pub open_height: BlockHeight,
    pub close_height: BlockHeight,
    pub arbitration_lock_block: Uint2,
    pub interest_attribution: Uint1,
    pub left_bill: AddrBalance,
    pub right_bill: AddrBalance,
    pub if_challenging: ChallengePeriodDataOptional,
    pub if_distribution: ClosedDistributionDataOptional,
}

fn check_channel_status(status: Uint1, if_challenging: bool, if_distribution: bool) -> Ret<()> {
    match status.uint() {
        0 => {
            if if_challenging || if_distribution {
                return errf!("channel opening status cannot carry extra data");
            }
        }
        1 => {
            if !if_challenging || if_distribution {
                return errf!("channel challenging status data mismatch");
            }
        }
        2 | 3 => {
            if if_challenging || !if_distribution {
                return errf!("channel closed status data mismatch");
            }
        }
        _ => return errf!("channel status invalid"),
    }
    Ok(())
}

impl Encode for ChannelSto {
    fn size(&self) -> usize {
        self.status.size()
            + self.reuse_version.size()
            + self.open_height.size()
            + self.close_height.size()
            + self.arbitration_lock_block.size()
            + self.interest_attribution.size()
            + self.left_bill.size()
            + self.right_bill.size()
            + self.if_challenging.size()
            + self.if_distribution.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.status.encode_to(out);
        self.reuse_version.encode_to(out);
        self.open_height.encode_to(out);
        self.close_height.encode_to(out);
        self.arbitration_lock_block.encode_to(out);
        self.interest_attribution.encode_to(out);
        self.left_bill.encode_to(out);
        self.right_bill.encode_to(out);
        self.if_challenging.encode_to(out);
        self.if_distribution.encode_to(out);
    }
}

impl Decode for ChannelSto {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut r = Reader::new(buf);
        let status: Uint1 = r.read()?;
        let reuse_version: Uint4 = r.read()?;
        let open_height: BlockHeight = r.read()?;
        let close_height: BlockHeight = r.read()?;
        let arbitration_lock_block: Uint2 = r.read()?;
        let interest_attribution: Uint1 = r.read()?;
        let left_bill: AddrBalance = r.read()?;
        let right_bill: AddrBalance = r.read()?;
        let if_challenging: ChallengePeriodDataOptional = r.read()?;
        let if_distribution: ClosedDistributionDataOptional = r.read()?;
        check_channel_status(
            status,
            if_challenging.is_exist(),
            if_distribution.is_exist(),
        )?;
        Ok((
            Self {
                status,
                reuse_version,
                open_height,
                close_height,
                arbitration_lock_block,
                interest_attribution,
                left_bill,
                right_bill,
                if_challenging,
                if_distribution,
            },
            r.used(),
        ))
    }
}
