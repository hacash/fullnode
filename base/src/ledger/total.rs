use field::{Decode, Encode, Reader, Uint8, Uint12};
use sys::Ret;

/// Statistics maintained by the base execution layer. Field order is part of the
/// state codec and must remain append-only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseTotal {
    pub tx_fee_burn90_238: Uint12,
    pub ast_vm_gas_burn_238: Uint12,
    pub contract_protocol_cost_burn_238: Uint12,
    pub contract_deploy_count: Uint8,
    pub contract_update_count: Uint8,
    pub contract_charge_bytes_total: Uint12,
    pub tx_fee_pay_total_238: Uint12,
    pub tx_fee_got_total_238: Uint12,
    pub blackhole_hac_burn_238: Uint12,
    pub blackhole_sat_burn: Uint8,
    pub blackhole_asset_burn_count: Uint8,
    pub blackhole_hacd_burn_count: Uint8,
}

impl Encode for BaseTotal {
    fn size(&self) -> usize {
        self.tx_fee_burn90_238.size()
            + self.ast_vm_gas_burn_238.size()
            + self.contract_protocol_cost_burn_238.size()
            + self.contract_deploy_count.size()
            + self.contract_update_count.size()
            + self.contract_charge_bytes_total.size()
            + self.tx_fee_pay_total_238.size()
            + self.tx_fee_got_total_238.size()
            + self.blackhole_hac_burn_238.size()
            + self.blackhole_sat_burn.size()
            + self.blackhole_asset_burn_count.size()
            + self.blackhole_hacd_burn_count.size()
    }

    fn encode_to(&self, out: &mut Vec<u8>) {
        self.tx_fee_burn90_238.encode_to(out);
        self.ast_vm_gas_burn_238.encode_to(out);
        self.contract_protocol_cost_burn_238.encode_to(out);
        self.contract_deploy_count.encode_to(out);
        self.contract_update_count.encode_to(out);
        self.contract_charge_bytes_total.encode_to(out);
        self.tx_fee_pay_total_238.encode_to(out);
        self.tx_fee_got_total_238.encode_to(out);
        self.blackhole_hac_burn_238.encode_to(out);
        self.blackhole_sat_burn.encode_to(out);
        self.blackhole_asset_burn_count.encode_to(out);
        self.blackhole_hacd_burn_count.encode_to(out);
    }
}

impl Decode for BaseTotal {
    fn decode(buf: &[u8]) -> Ret<(Self, usize)> {
        let mut reader = Reader::new(buf);
        Ok((
            Self {
                tx_fee_burn90_238: reader.read()?,
                ast_vm_gas_burn_238: reader.read()?,
                contract_protocol_cost_burn_238: reader.read()?,
                contract_deploy_count: reader.read()?,
                contract_update_count: reader.read()?,
                contract_charge_bytes_total: reader.read()?,
                tx_fee_pay_total_238: reader.read()?,
                tx_fee_got_total_238: reader.read()?,
                blackhole_hac_burn_238: reader.read()?,
                blackhole_sat_burn: reader.read()?,
                blackhole_asset_burn_count: reader.read()?,
                blackhole_hacd_burn_count: reader.read()?,
            },
            reader.used(),
        ))
    }
}
