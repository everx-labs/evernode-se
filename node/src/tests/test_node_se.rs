use crate::blockchain_config_from_json;
use ever_block::{GasLimitsPrices, MsgForwardPrices};

#[test]
fn test_blockchain_config_parsing() -> ever_block::Result<()> {
    let blockchain_config = blockchain_config_from_json(include_str!("blockchain.conf.json"))?;

    let gas_price = 10_000;
    let flat_gas_limit = 1_000;
    let mut expected = GasLimitsPrices {
        gas_price: gas_price << 16,
        flat_gas_limit,
        flat_gas_price: flat_gas_limit * gas_price,
        gas_limit: 1_000_000,
        special_gas_limit: 100_000_000,
        gas_credit: 10_000,
        block_gas_limit: 11_000_000,
        freeze_due_limit: 100_000_000,
        delete_due_limit: 1_000_000_000,
        max_gas_threshold: 0,
    };

    expected.max_gas_threshold = expected.calc_max_gas_threshold();

    assert_eq!(blockchain_config.get_gas_config(true), &expected);

    let gas_price = 1_000;
    let flat_gas_limit = 1_000;
    let mut expected = GasLimitsPrices {
        gas_price: gas_price << 16,
        flat_gas_limit,
        flat_gas_price: flat_gas_limit * gas_price,
        gas_limit: 1_000_000,
        special_gas_limit: 1_000_000,
        gas_credit: 10_000,
        block_gas_limit: 10_000_000,
        freeze_due_limit: 100_000_000,
        delete_due_limit: 1_000_000_000,
        max_gas_threshold: 0,
    };

    expected.max_gas_threshold = expected.calc_max_gas_threshold();

    assert_eq!(blockchain_config.get_gas_config(false), &expected);

    assert_eq!(
        blockchain_config.get_fwd_prices(true),
        &MsgForwardPrices {
            lump_price: 10_000_000,
            bit_price: 10_000 << 16,
            cell_price: 1_000_000 << 16,
            ihr_price_factor: 98_304,
            first_frac: 21_845,
            next_frac: 21_845,
        }
    );

    assert_eq!(
        blockchain_config.get_fwd_prices(false),
        &MsgForwardPrices {
            lump_price: 1_000_000,
            bit_price: 1_000 << 16,
            cell_price: 100_000 << 16,
            ihr_price_factor: 98_304,
            first_frac: 21_845,
            next_frac: 21_845,
        }
    );

    Ok(())
}
