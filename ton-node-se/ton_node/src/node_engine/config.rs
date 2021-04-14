use adnl::config::AdnlServerConfigJson;
use ed25519_dalek::PublicKey;
//use poa::engines::validator_set::PublicKeyListImport;
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Default)]
pub struct KafkaConfig {
    pub brokers: String,
    pub group_id: String,
    pub input_topic: String,
    pub output_blocks_topic: String,
    pub output_msgs_topic: String,
    pub output_trans_topic: String,
    pub output_accounts_topic: String,
}

#[derive(Deserialize, Default)]
pub struct ShardIdConfig {
    pub workchain: i32,
    pub shardchain_pfx: u64,
    pub shardchain_pfx_len: u8,
}

impl ShardIdConfig {
    pub fn shard_ident(&self) -> ton_block::ShardIdent {
        ton_block::ShardIdent::with_prefix_len(self.shardchain_pfx_len as u8, self.workchain, self.shardchain_pfx as u64).unwrap()
    }
}

/// Node config importer from JSON
#[derive(Deserialize)]
pub struct NodeConfig {
    pub node_index: u8,
    pub poa_validators: u16,
    pub poa_interval: u16,
    pub port: u16,
    pub private_key: String,
    keys: Vec<String>,
    pub boot: Vec<String>,
    shard_id: ShardIdConfig,
    pub adnl: AdnlServerConfigJson,
    kafka_msg_recv: serde_json::Value,
    document_db: serde_json::Value,
}

impl NodeConfig {

    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// Import key values from list of files
    pub fn import_keys(&self) -> Result<Vec<PublicKey>, String> {
        let mut ret = Vec::new();
        for path in self.keys.iter() {
            let data = fs::read(Path::new(path))
                .map_err(|e| format!("Error reading key file {}, {}", path, e))?;
            ret.push(
                PublicKey::from_bytes(&data)
                    .map_err(|e| format!("Cannot import key from {}, {}", path, e))?,
            );
        }
        Ok(ret)
    }

    pub fn shard_id_config(&self) -> &ShardIdConfig {
        &self.shard_id
    }

    pub fn kafka_msg_recv_config(&self) -> String {
        // If it has read to serde_json::Value it must be transformed to string
        serde_json::to_string(&self.kafka_msg_recv).unwrap()
    }

    pub fn document_db_config(&self) -> String {
        // If it has read to serde_json::Value it must be transformed to string
        serde_json::to_string(&self.document_db).unwrap()
    }
}

pub mod blockchain {
    use serde::Deserialize;
    use ton_executor::BlockchainConfig;
    use ton_block::{
        GlobalVersion, StoragePrices, GasLimitsPrices, MsgForwardPrices, ConfigParams,
        ConfigParamEnum, ConfigParam18, ConfigParam31, FundamentalSmcAddresses, ConfigParam8
    };
    use ton_types::UInt256;

    macro_rules! serde_mirror {
        ($name:ident, $mirror_name:ident {
            $($field_name:ident: $field_type:ty,)*
        }) => {
            #[derive(Deserialize)]
            struct $mirror_name {
                $($field_name: $field_type,)*
            }
            impl Into<$name> for $mirror_name {
                fn into(self) -> $name {
                    $name {
                        $($field_name: self.$field_name,)*
                    }
                }
            }
        }
    }

    trait ToFixedPoint {
        fn to_fixed_point(&self) -> u64;
    }

    impl ToFixedPoint for f64 {
        #[inline]
        fn to_fixed_point(&self) -> u64 {
            (self * (1 << 16) as f64).round() as u64
        }
    }

    serde_mirror! {
        StoragePrices, StoragePricesJson {
            utime_since: u32,
            bit_price_ps: u64,
            cell_price_ps: u64,
            mc_bit_price_ps: u64,
            mc_cell_price_ps: u64,
        }
    }

    #[derive(Deserialize)]
    struct GasLimitsPricesJson {
        gas_price: f64,
        flat_gas_limit: u64,
        gas_limit: u64,
        special_gas_limit: u64,
        gas_credit: u64,
        block_gas_limit: u64,
        freeze_due_limit: u64,
        delete_due_limit: u64,
    }

    impl Into<GasLimitsPrices> for GasLimitsPricesJson {
        fn into(self) -> GasLimitsPrices {
            let mut result = GasLimitsPrices {
                gas_price: self.gas_price.to_fixed_point(),
                gas_limit: self.gas_limit,
                special_gas_limit: self.special_gas_limit,
                gas_credit: self.gas_credit,
                block_gas_limit: self.block_gas_limit,
                freeze_due_limit: self.freeze_due_limit,
                delete_due_limit: self.delete_due_limit,
                flat_gas_limit: self.flat_gas_limit,
                flat_gas_price: (self.flat_gas_limit as f64 * self.gas_price).round() as u64,
                max_gas_threshold: 0
            };

            result.max_gas_threshold = result.calc_max_gas_threshold();

            result
        }
    }

    #[derive(Deserialize)]
    struct GasPricesJson {
        mc: GasLimitsPricesJson,
        wc: GasLimitsPricesJson,
    }

    #[derive(Deserialize)]
    struct MsgForwardPricesJson {
        lump_price: u64,
        bit_price: f64,
        cell_price: f64,
        ihr_price_factor: f64,
        first_frac: f64,
        next_frac: f64,
    }

    impl Into<MsgForwardPrices> for MsgForwardPricesJson {
        fn into(self) -> MsgForwardPrices {
            MsgForwardPrices {
                lump_price: self.lump_price,
                bit_price: self.bit_price.to_fixed_point(),
                cell_price: self.cell_price.to_fixed_point(),
                ihr_price_factor: self.ihr_price_factor.to_fixed_point() as u32,
                first_frac: self.first_frac.to_fixed_point() as u16,
                next_frac: self.next_frac.to_fixed_point() as u16,
            }
        }
    }

    #[derive(Deserialize)]
    struct ForwardPricesJson {
        mc: MsgForwardPricesJson,
        wc: MsgForwardPricesJson,
    }

    serde_mirror! {
        GlobalVersion, GlobalVersionJson {
            version: u32,
            capabilities: u64,
        }
    }

    #[derive(Deserialize)]
    struct BlockchainConfigJson {
        storage_prices: Vec<StoragePricesJson>,
        gas_prices: GasPricesJson,
        forward_prices: ForwardPricesJson,
        fundamental_smc_addresses: Vec<String>,
        global_version: GlobalVersionJson,
    }

    impl BlockchainConfigJson {
        pub fn convert_to_config_params(self) -> ton_types::Result<ConfigParams> {
            let mut result = ConfigParams::default();

            let mut config_param_18 = ConfigParam18::default();
            for price in self.storage_prices.into_iter() {
                config_param_18.insert(&price.into())?;
            }
            result.set_config(ConfigParamEnum::ConfigParam18(config_param_18))?;

            result.set_config(ConfigParamEnum::ConfigParam20(self.gas_prices.mc.into()))?;
            result.set_config(ConfigParamEnum::ConfigParam21(self.gas_prices.wc.into()))?;

            result.set_config(ConfigParamEnum::ConfigParam24(self.forward_prices.mc.into()))?;
            result.set_config(ConfigParamEnum::ConfigParam25(self.forward_prices.wc.into()))?;

            let mut fundamental_smc_addr = FundamentalSmcAddresses::default();
            for addr in self.fundamental_smc_addresses {
                fundamental_smc_addr.add_key(&UInt256::from_str(&addr)?)?;
            }

            result.set_config(ConfigParamEnum::ConfigParam31(ConfigParam31 {
                fundamental_smc_addr
            }))?;

            result.set_config(ConfigParamEnum::ConfigParam8(ConfigParam8 {
                global_version: self.global_version.into()
            }))?;

            Ok(result)
        }
    }

    pub fn blockchain_config_from_json(json: &str) -> ton_types::Result<BlockchainConfig> {
        let blockchain_config_json: BlockchainConfigJson = serde_json::from_str(json)?;
        let config_params = blockchain_config_json.convert_to_config_params()?;
        BlockchainConfig::with_config(config_params)
    }
}
