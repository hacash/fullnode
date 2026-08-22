//! `tx.estimate_fee`: offline fee guidance (Unified SDK 2.0, doc 14 §4.7).
//! The chain's gas model bills type-3 txs `max(declared_fee, floor * billing_size)`
//! in the 238 sub-unit; type-2 txs carry no purity floor (a non-zero fee within
//! the size rule suffices). This operation computes the floor at a height, the
//! tx's billing size, the type-3 minimum fee, and whether the declared fee
//! clears its type's bar. The SDK never executes or consults a node.

use field::{Amount, UNIT_238};

use crate::error::{SdkError, SdkErrorCode};
use crate::json::SdkJsonTo;
use crate::schema::SCHEMA_FEE_ESTIMATE;

/// `tx.estimate_fee` response (`hacash.sdk/fee-estimate@1`).
#[derive(Debug, Clone, PartialEq)]
pub struct FeeEstimate {
    pub schema: String,
    pub tx_type: u8,
    /// The height the floor was evaluated at (`0` when the caller omits it:
    /// the initial floor, the highest one, so a wallet never underbids).
    pub height: u64,
    pub fee_purity_floor: u64,
    pub billing_size: usize,
    /// `floor * billing_size` in the 238 sub-unit, formatted as a fin string;
    /// `None` for tx types without a purity floor (type-2).
    pub minimum_fee: Option<String>,
    pub fee: String,
    pub fee_purity: u64,
    /// Type-3: `fee_purity >= floor`. Type-2: the fee is non-zero (the chain
    /// has no purity bar for it).
    pub fee_enough: bool,
}

/// `tx.estimate_fee`: decode one tx body and evaluate its fee against the
/// purity floor at `height`.
pub fn estimate_fee(body_hex: &str, height: Option<u64>) -> Result<FeeEstimate, SdkError> {
    let body = crate::inspect::decode_body_hex(body_hex)?;
    let tx = crate::inspect::decode_tx(&body)?;
    let height = height.unwrap_or(0);
    let profile = crate::service::profile();
    let floor = profile.protocol_params.fee_purity_floor_at(height);
    let billing_size = tx.billing_size().map_err(SdkError::from)?;
    let tx_type = tx.ty();
    let minimum_fee = if tx_type == hacash_params::TX_TYPE_3 {
        let floor_fee = (floor as u128)
            .checked_mul(billing_size as u128)
            .ok_or_else(|| {
                SdkError::new(SdkErrorCode::ParseFailed, "fee floor * billing size overflow")
            })?;
        Some(Amount::coin_u128(floor_fee, UNIT_238).to_fin_string())
    } else {
        None
    };
    let fee = tx.fee().to_fin_string();
    let fee_purity = tx.fee_purity();
    let fee_enough = if tx_type == hacash_params::TX_TYPE_3 {
        fee_purity >= floor
    } else {
        !tx.fee().is_zero()
    };
    Ok(FeeEstimate {
        schema: SCHEMA_FEE_ESTIMATE.to_owned(),
        tx_type,
        height,
        fee_purity_floor: floor,
        billing_size,
        minimum_fee,
        fee,
        fee_purity,
        fee_enough,
    })
}

impl SdkJsonTo for FeeEstimate {
    fn to_json_string(&self) -> String {
        use crate::json::{kv, obj, q, qnum};
        obj(vec![
            kv("schema", q(&self.schema)),
            kv("tx_type", qnum(self.tx_type as u64)),
            kv("height", qnum(self.height)),
            kv("fee_purity_floor", qnum(self.fee_purity_floor)),
            kv("billing_size", qnum(self.billing_size as u64)),
            kv(
                "minimum_fee",
                self.minimum_fee.as_deref().map(q).unwrap_or_else(|| "null".to_owned()),
            ),
            kv("fee", q(&self.fee)),
            kv("fee_purity", qnum(self.fee_purity)),
            kv("fee_enough", if self.fee_enough { "true".to_owned() } else { "false".to_owned() }),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{ActionSpec, TransactionSpec, build_transaction};
    use crate::spec_codec::WireValue;

    const MAIN: &str = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

    fn sample(tx_type: u8) -> TransactionSpec {
        TransactionSpec {
            schema: None,
            tx_type,
            main: MAIN.to_owned(),
            fee: "1:244".to_owned(),
            timestamp: Some(1_755_223_764),
            gas_max: None,
            actions: vec![ActionSpec::new(
                "transfer_hac_to",
                vec![
                    ("to".to_owned(), WireValue::Str(MAIN.to_owned())),
                    ("hacash".to_owned(), WireValue::Str("12:244".to_owned())),
                ],
            )],
        }
    }

    #[test]
    fn type2_has_no_purity_floor() {
        let built = build_transaction(&sample(2)).unwrap();
        let estimate = estimate_fee(&built.body, None).unwrap();
        assert_eq!(estimate.tx_type, 2);
        assert_eq!(estimate.height, 0);
        assert_eq!(
            estimate.fee_purity_floor,
            crate::service::profile()
                .protocol_params
                .fee_purity_floor_at(0)
        );
        assert!(estimate.billing_size > 0);
        assert_eq!(estimate.minimum_fee, None);
        assert_eq!(estimate.fee, "1:244");
        assert!(estimate.fee_enough); // type-2: any non-zero fee
    }

    #[test]
    fn type3_minimum_fee_and_floor_check() {
        let built = build_transaction(&sample(3)).unwrap();
        let estimate = estimate_fee(&built.body, Some(9)).unwrap();
        assert_eq!(estimate.tx_type, 3);
        // Mainnet's floor is 50,000 with no height reductions (the 100-floor
        // schedule in base's registry tests is a fixture, not mainnet).
        assert_eq!(estimate.fee_purity_floor, 50_000);
        let minimum = estimate.minimum_fee.as_deref().unwrap();
        assert!(!minimum.is_empty());
        assert!(!estimate.fee_enough); // 1:244 purity is far below the floor
    }

    #[test]
    fn a_sufficient_type3_fee_is_reported_enough() {
        let mut spec = sample(3);
        // The 238-unit fee value must clear floor * billing_size: use a huge fee.
        spec.fee = "1000000:244".to_owned();
        let built = build_transaction(&spec).unwrap();
        let estimate = estimate_fee(&built.body, Some(9)).unwrap();
        assert_eq!(estimate.fee_purity_floor, 50_000);
        assert!(estimate.fee_enough);
    }

    #[test]
    fn rejects_a_bad_body() {
        let error = estimate_fee("aabb", None).unwrap_err();
        assert_eq!(error.code, "parse_failed");
    }
}
