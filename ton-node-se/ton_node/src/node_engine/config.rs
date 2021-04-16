use adnl::config::AdnlServerConfigJson;
use ed25519_dalek::PublicKey;
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

