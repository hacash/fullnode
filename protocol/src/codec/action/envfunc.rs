//! VM syscall actions: ACTENV (0x07xx) / ACTVIEW (0x06xx), invoked via
//! `Context::action_call` with kid = `[0x07|0x06, idx]` where idx = KIND % 256.

use field::{Address, DiamondName, DiamondNameListMax200, DiamondNumber, Fold64, Uint1, Uint2};

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct EnvHeight {
    pub kind: Uint2,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct EnvMainAddr {
    pub kind: Uint2,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct EnvBlockAuthorAddr {
    pub kind: Uint2,
}
#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct EnvMessageNum { pub kind: Uint2 }
#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct EnvBlobNum { pub kind: Uint2 }

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewBalance {
    pub kind: Uint2,
    pub addr: Address,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewAssetBalance {
    pub kind: Uint2,
    pub addr: Address,
    pub serial: Fold64,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewCheckSign {
    pub kind: Uint2,
    pub addr: Address,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewDiaInscNum {
    pub kind: Uint2,
    pub diamond: DiamondName,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewDiaInscGet {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub inscidx: Uint1,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewDiaNameList {
    pub kind: Uint2,
    pub addr: Address,
    pub page: DiamondNumber,
    pub limit: DiamondNumber,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewDiaOwnerAddrs {
    pub kind: Uint2,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewMessage {
    pub kind: Uint2,
    pub idx: Uint1,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewBlob {
    pub kind: Uint2,
    pub idx: Uint1,
    pub start: Uint2,
    pub end: Uint2,
}
#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct ViewBlobSize { pub kind: Uint2, pub idx: Uint1 }

impl EnvHeight {
    pub const KIND: u16 = 0x0701;
}

impl EnvMainAddr {
    pub const KIND: u16 = 0x0702;
}

impl EnvBlockAuthorAddr {
    pub const KIND: u16 = 0x0703;
}
impl EnvMessageNum { pub const KIND: u16 = 0x0704; }
impl EnvBlobNum { pub const KIND: u16 = 0x0705; }

impl ViewBalance {
    pub const KIND: u16 = 0x0601;
}

impl ViewAssetBalance {
    pub const KIND: u16 = 0x0602;
}

impl ViewCheckSign {
    pub const KIND: u16 = 0x0609;
}

impl ViewDiaInscNum {
    pub const KIND: u16 = 0x0611;
}

impl ViewDiaInscGet {
    pub const KIND: u16 = 0x0612;
}

impl ViewDiaNameList {
    pub const KIND: u16 = 0x0613;
}

impl ViewDiaOwnerAddrs {
    pub const KIND: u16 = 0x0614;
}

impl ViewMessage { pub const KIND: u16 = 0x0615; }
impl ViewBlob { pub const KIND: u16 = 0x0616; }
impl ViewBlobSize { pub const KIND: u16 = 0x0617; }

base::impl_action_facts! {
    EnvHeight {
        name: "block_height",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |_: &EnvHeight| "Syscall: Get block height".to_owned(),

    }
}

base::impl_action_facts! { ViewMessage {
    name: "tx_message", scope: base::ActScope::CALL_ONLY, min_tx_type: 3,
    description: |this: &ViewMessage| format!("Syscall: Get transaction message {}", this.idx.uint()),
} }

base::impl_action_facts! { EnvMessageNum {
    name: "tx_message_num", scope: base::ActScope::CALL_ONLY, min_tx_type: 3,
    description: |_: &EnvMessageNum| "Syscall: Get transaction message count".to_owned(),
} }
base::impl_action_facts! { EnvBlobNum {
    name: "tx_blob_num", scope: base::ActScope::CALL_ONLY, min_tx_type: 3,
    description: |_: &EnvBlobNum| "Syscall: Get transaction blob count".to_owned(),
} }
base::impl_action_facts! { ViewBlobSize {
    name: "tx_blob_size", scope: base::ActScope::CALL_ONLY, min_tx_type: 3,
    description: |this: &ViewBlobSize| format!("Syscall: Get transaction blob {} size", this.idx.uint()),
} }

base::impl_action_facts! { ViewBlob {
    name: "tx_blob", scope: base::ActScope::CALL_ONLY, min_tx_type: 3,
    description: |this: &ViewBlob| format!("Syscall: Get transaction blob {} [{}..{}]", this.idx.uint(), this.start.uint(), this.end.uint()),
} }

base::impl_action_facts! {
    EnvMainAddr {
        name: "tx_main_addr",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |_: &EnvMainAddr| "Syscall: Get main address".to_owned(),

    }
}

base::impl_action_facts! {
    EnvBlockAuthorAddr {
        name: "block_author_addr",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |_: &EnvBlockAuthorAddr| "Syscall: Get author address".to_owned(),

    }
}

base::impl_action_facts! {
    ViewBalance {
        name: "balance",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewBalance| format!("Syscall: Get balance for {}", this.addr.to_readable()),

    }
}

base::impl_action_facts! {
    ViewAssetBalance {
        name: "asset_balance",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewAssetBalance| format!("Syscall: Get asset {} balance for {}", this.serial.uint(), this.addr.to_readable()),

    }
}

base::impl_action_facts! {
    ViewCheckSign {
        name: "check_signature",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewCheckSign| format!("Syscall: Check signature for {}", this.addr.to_readable()),

    }
}

base::impl_action_facts! {
    ViewDiaInscNum {
        name: "hacd_insc_num",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaInscNum| format!("Syscall: Get diamond inscription number for <{}>", this.diamond.to_readable()),

    }
}

base::impl_action_facts! {
    ViewDiaInscGet {
        name: "hacd_insc_get",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaInscGet| format!("Syscall: Get diamond inscription data for <{}>", this.diamond.to_readable()),

    }
}

base::impl_action_facts! {
    ViewDiaNameList {
        name: "hacd_name_list",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaNameList| format!("Syscall: Get HACD name list for {} page {} limit {}", this.addr.to_readable(), this.page.uint(), this.limit.uint()),

    }
}

base::impl_action_facts! {
    ViewDiaOwnerAddrs {
        name: "hacd_owner_addrs",
        scope: base::ActScope::CALL_ONLY,
        min_tx_type: 3,
        description: |this: &ViewDiaOwnerAddrs| format!("Syscall: Get HACD owner addresses for {}", this.diamonds.splitstr()),

    }
}
