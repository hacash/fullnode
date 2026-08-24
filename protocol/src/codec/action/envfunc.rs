//! VM syscall actions: ACTENV (0x07xx) / ACTVIEW (0x06xx), invoked via
//! `Context::action_call` with kid = `[0x07|0x06, idx]` where idx = KIND % 256.

use field::{Address, DiamondName, DiamondNameListMax200, DiamondNumber, Fold64, Uint1, Uint2};

base::action_simple! { EnvHeight, 0x0701, 3, CALL_ONLY, {
}, this, {
    name: "block_height",
    description: "Syscall: Get block height".to_owned()
}}
base::action_simple! { EnvMainAddr, 0x0702, 3, CALL_ONLY, {
}, this, {
    name: "tx_main_addr",
    description: "Syscall: Get main address".to_owned()
}}
base::action_simple! { EnvBlockAuthorAddr, 0x0703, 3, CALL_ONLY, {
}, this, {
    name: "block_author_addr",
    description: "Syscall: Get author address".to_owned()
}}
base::action_simple! { EnvMessageNum, 0x0704, 3, CALL_ONLY, {
}, this, {
    name: "tx_message_num",
    description: "Syscall: Get transaction message count".to_owned()
}}
base::action_simple! { EnvBlobNum, 0x0705, 3, CALL_ONLY, {
}, this, {
    name: "tx_blob_num",
    description: "Syscall: Get transaction blob count".to_owned()
}}
base::action_simple! { ViewBalance, 0x0601, 3, CALL_ONLY, {
    addr: Address
}, this, {
    name: "balance",
    description: format!("Syscall: Get balance for {}", this.addr.to_readable())
}}
base::action_simple! { ViewAssetBalance, 0x0602, 3, CALL_ONLY, {
    addr: Address,
    serial: Fold64
}, this, {
    name: "asset_balance",
    description: format!("Syscall: Get asset {} balance for {}", this.serial.uint(), this.addr.to_readable())
}}
base::action_simple! { ViewCheckSign, 0x0609, 3, CALL_ONLY, {
    addr: Address
}, this, {
    name: "check_signature",
    description: format!("Syscall: Check signature for {}", this.addr.to_readable())
}}
base::action_simple! { ViewDiaInscNum, 0x0611, 3, CALL_ONLY, {
    diamond: DiamondName
}, this, {
    name: "hacd_insc_num",
    description: format!("Syscall: Get diamond inscription number for <{}>", this.diamond.to_readable())
}}
base::action_simple! { ViewDiaInscGet, 0x0612, 3, CALL_ONLY, {
    diamond: DiamondName,
    inscidx: Uint1
}, this, {
    name: "hacd_insc_get",
    description: format!("Syscall: Get diamond inscription data for <{}>", this.diamond.to_readable())
}}
base::action_simple! { ViewDiaNameList, 0x0613, 3, CALL_ONLY, {
    addr: Address,
    page: DiamondNumber,
    limit: DiamondNumber
}, this, {
    name: "hacd_name_list",
    description: format!("Syscall: Get HACD name list for {} page {} limit {}", this.addr.to_readable(), this.page.uint(), this.limit.uint())
}}
base::action_simple! { ViewDiaOwnerAddrs, 0x0614, 3, CALL_ONLY, {
    diamonds: DiamondNameListMax200
}, this, {
    name: "hacd_owner_addrs",
    description: format!("Syscall: Get HACD owner addresses for {}", this.diamonds.splitstr())
}}
base::action_simple! { ViewMessage, 0x0615, 3, CALL_ONLY, {
    idx: Uint1
}, this, {
    name: "tx_message",
    description: format!("Syscall: Get transaction message {}", this.idx.uint())
}}
base::action_simple! { ViewBlob, 0x0616, 3, CALL_ONLY, {
    idx: Uint1,
    start: Uint2,
    end: Uint2
}, this, {
    name: "tx_blob",
    description: format!("Syscall: Get transaction blob {} [{}..{}]", this.idx.uint(), this.start.uint(), this.end.uint())
}}
base::action_simple! { ViewBlobSize, 0x0617, 3, CALL_ONLY, {
    idx: Uint1
}, this, {
    name: "tx_blob_size",
    description: format!("Syscall: Get transaction blob {} size", this.idx.uint())
}}
