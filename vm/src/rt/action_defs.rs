/// ACTION / ACTENV / ACTVIEW host-id display definitions for the fitsh
/// decompiler (`lang::Formater`). Each entry is (id, display name, return
/// value type, argument count).
///
/// The numeric ids are the low byte of the protocol `HacToTrs` / `EnvHeight` /
/// `ViewBalance` KIND constants (`protocol::codec::action`, `mint` for
/// `DiaInscEdit`). They are hardcoded here because the VM cannot depend on
/// protocol/mint; keep in sync with the protocol registration tables.
pub type ActDefTy = (u8, &'static str, ValueTy, usize);

pub const ACTION_DEFS: [ActDefTy; 14] = [
    (0x01, "transfer_hac_to", ValueTy::Nil, 2),
    (0x0d, "transfer_hac_from", ValueTy::Nil, 2),
    (0x0e, "transfer_hac_from_to", ValueTy::Nil, 3),
    (0x0a, "transfer_sat_to", ValueTy::Nil, 2),
    (0x0b, "transfer_sat_from", ValueTy::Nil, 2),
    (0x0c, "transfer_sat_from_to", ValueTy::Nil, 3),
    (0x05, "transfer_hacd_single_to", ValueTy::Nil, 2),
    (0x07, "transfer_hacd_to", ValueTy::Nil, 2),
    (0x08, "transfer_hacd_from", ValueTy::Nil, 2),
    (0x06, "transfer_hacd_from_to", ValueTy::Nil, 3),
    (0x22, "hacd_insc_edit", ValueTy::Nil, 5),
    (0x11, "transfer_asset_to", ValueTy::Nil, 2),
    (0x12, "transfer_asset_from", ValueTy::Nil, 2),
    (0x13, "transfer_asset_from_to", ValueTy::Nil, 3),
];

pub const ACTION_ENV_DEFS: [ActDefTy; 3] = [
    (0x01, "block_height", ValueTy::U64, 0),
    (0x02, "tx_main_addr", ValueTy::Address, 0),
    (0x03, "block_author_addr", ValueTy::Address, 0),
];

pub const ACTION_VIEW_DEFS: [ActDefTy; 7] = [
    (0x01, "balance", ValueTy::Bytes, 1),
    (0x02, "asset_balance", ValueTy::U64, 2),
    (0x09, "check_signature", ValueTy::Bool, 1),
    (0x11, "hacd_insc_num", ValueTy::U8, 1),
    (0x12, "hacd_insc_get", ValueTy::Bytes, 2),
    (0x13, "hacd_name_list", ValueTy::Bytes, 3),
    (0x14, "hacd_owner_addrs", ValueTy::Bytes, 1),
];

pub fn search_act_by_id<'a>(id: u8, exts: &'a [ActDefTy]) -> Option<&'a ActDefTy> {
    exts.iter().find(|def| def.0 == id)
}

pub fn search_act_name_by_id(id: u8, exts: &[ActDefTy]) -> &'static str {
    match search_act_by_id(id, exts) {
        Some(def) => def.1,
        _ => "__unknown__",
    }
}
