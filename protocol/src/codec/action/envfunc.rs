//! VM syscall actions: ACTENV (0x07xx) / ACTVIEW (0x06xx), invoked via
//! `Context::action_call` with kid = `[0x07|0x06, idx]` where idx = KIND % 256.

use field::{Address, DiamondName, DiamondNameListMax200, DiamondNumber, Fold64, Uint1, Uint2};

base::action_simple! { EnvHeight, 0x0701, 3, CALL_ONLY, {
}, this, {
    name: "block_height",
    description: "Syscall: Get block height".to_owned()
}}
base::action_simple! { TxMainAddr, 0x0702, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get main address".to_owned()
}}
base::action_simple! { BlockAuthorAddr, 0x0703, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get author address".to_owned()
}}
base::action_simple! { TxMessageNum, 0x0704, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get transaction message count".to_owned()
}}
base::action_simple! { TxBlobNum, 0x0705, 3, CALL_ONLY, {
}, this, {
    description: "Syscall: Get transaction blob count".to_owned()
}}
base::action_simple! { BalanceCoin, 0x0601, 3, CALL_ONLY, {
    addr: Address
}, this, {
    description: format!("Syscall: Get balance for {}", this.addr.to_readable())
}}
base::action_simple! { BalanceAsset, 0x0602, 3, CALL_ONLY, {
    addr: Address,
    serial: Fold64
}, this, {
    description: format!("Syscall: Get asset {} balance for {}", this.serial.uint(), this.addr.to_readable())
}}
base::action_simple! { CheckSignature, 0x0609, 3, CALL_ONLY, {
    addr: Address
}, this, {
    description: format!("Syscall: Check signature for {}", this.addr.to_readable())
}}
base::action_simple! { HacdInscNum, 0x0611, 3, CALL_ONLY, {
    diamond: DiamondName
}, this, {
    description: format!("Syscall: Get diamond inscription number for <{}>", this.diamond.to_readable())
}}
base::action_simple! { HacdInscGet, 0x0612, 3, CALL_ONLY, {
    diamond: DiamondName,
    inscidx: Uint1
}, this, {
    description: format!("Syscall: Get diamond inscription data for <{}>", this.diamond.to_readable())
}}
base::action_simple! { HacdNameList, 0x0613, 3, CALL_ONLY, {
    addr: Address,
    page: DiamondNumber,
    limit: DiamondNumber
}, this, {
    description: format!("Syscall: Get HACD name list for {} page {} limit {}", this.addr.to_readable(), this.page.uint(), this.limit.uint())
}}
base::action_simple! { HacdOwnerAddrs, 0x0614, 3, CALL_ONLY, {
    diamonds: DiamondNameListMax200
}, this, {
    description: format!("Syscall: Get HACD owner addresses for {}", this.diamonds.splitstr())
}}
base::action_simple! { TxMessage, 0x0615, 3, CALL_ONLY, {
    idx: Uint1
}, this, {
    description: format!("Syscall: Get transaction message {}", this.idx.uint())
}}
base::action_simple! { TxBlob, 0x0616, 3, CALL_ONLY, {
    idx: Uint1,
    start: Uint2,
    end: Uint2
}, this, {
    description: format!("Syscall: Get transaction blob {} [{}..{}]", this.idx.uint(), this.start.uint(), this.end.uint())
}}
base::action_simple! { TxBlobSize, 0x0617, 3, CALL_ONLY, {
    idx: Uint1
}, this, {
    description: format!("Syscall: Get transaction blob {} size", this.idx.uint())
}}
