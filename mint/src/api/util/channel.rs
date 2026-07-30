use field::{Address, Balance, ChannelId, ChannelSto, HacSat};
use sys::ToHex;

use super::request::json_string;

pub(crate) fn bill_json(address: &Address, balance: &Balance, unit: &str) -> String {
    format!(
        "{{\"address\":{},\"hacash\":{},\"satoshi\":{}}}",
        json_string(&address.to_readable()),
        json_string(&balance.hacash.to_unit_string(unit)),
        balance.satoshi.uint()
    )
}

pub(crate) fn hsat_json(bill: &HacSat, unit: &str) -> String {
    format!(
        "{{\"hacash\":{},\"satoshi\":{}}}",
        json_string(&bill.amount.to_unit_string(unit)),
        bill.satoshi.uint()
    )
}

pub(crate) fn channel_json(id: &ChannelId, channel: &ChannelSto, unit: &str) -> sys::Ret<String> {
    let mut fields = vec![
        format!("\"id\":{}", json_string(&id.as_ref().to_hex())),
        format!("\"status\":{}", channel.status.uint()),
        format!("\"open_height\":{}", channel.open_height.uint()),
        format!("\"close_height\":{}", channel.close_height.uint()),
        format!("\"reuse_version\":{}", channel.reuse_version.uint()),
        format!(
            "\"arbitration_lock\":{}",
            channel.arbitration_lock_block.uint()
        ),
        format!(
            "\"interest_attribution\":{}",
            channel.interest_attribution.uint()
        ),
        format!(
            "\"left\":{}",
            bill_json(&channel.left_bill.address, &channel.left_bill.balance, unit)
        ),
        format!(
            "\"right\":{}",
            bill_json(
                &channel.right_bill.address,
                &channel.right_bill.balance,
                unit
            )
        ),
    ];

    if channel.if_challenging.is_exist() {
        let challenging = channel.if_challenging.value();
        let is_left = challenging.assert_address_is_left_or_right.is_true();
        let assaddr = if is_left {
            channel.left_bill.address.to_readable()
        } else {
            channel.right_bill.address.to_readable()
        };
        fields.push(format!(
            concat!(
                "\"challenging\":{{",
                "\"launch_height\":{},",
                "\"assert_bill_auto_number\":{},",
                "\"assert_address_is_left_or_right\":{},",
                "\"assert_bill\":{{",
                "\"address\":{},",
                "\"hacash\":{},",
                "\"satoshi\":{}",
                "}}",
                "}}"
            ),
            challenging.challenge_launch_height.uint(),
            challenging.assert_bill_auto_number.uint(),
            is_left,
            json_string(&assaddr),
            json_string(&challenging.assert_bill.amount.to_unit_string(unit)),
            challenging.assert_bill.satoshi.uint()
        ));
    }

    if channel.if_distribution.is_exist() {
        let distribution = channel.if_distribution.value();
        let (final_left, final_right) = crate::genesis::calculate_interest_of_height(
            channel.close_height.uint(),
            channel.open_height.uint(),
            channel.interest_attribution,
            &distribution.left_bill.hacash,
            &distribution.right_bill.hacash,
        )?;
        fields.push(format!(
            "\"distribution\":{{\"left\":{},\"right\":{}}}",
            hsat_json(
                &HacSat {
                    amount: distribution.left_bill.hacash.clone(),
                    satoshi: distribution.left_bill.satoshi,
                },
                unit
            ),
            hsat_json(
                &HacSat {
                    amount: distribution.right_bill.hacash.clone(),
                    satoshi: distribution.right_bill.satoshi,
                },
                unit
            )
        ));
        fields.push(format!(
            "\"final_arrival\":{{\"left\":{},\"right\":{}}}",
            hsat_json(
                &HacSat {
                    amount: final_left,
                    satoshi: distribution.left_bill.satoshi,
                },
                unit
            ),
            hsat_json(
                &HacSat {
                    amount: final_right,
                    satoshi: distribution.right_bill.satoshi,
                },
                unit
            )
        ));
    }

    Ok(format!("{{\"ret\":0,{}}}", fields.join(",")))
}
