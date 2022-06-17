/*
* Copyright 2018-2022 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.  You may obtain a copy of the
* License at:
*
* https://www.ton.dev/licenses
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and limitations
* under the License.
*/

#[cfg(test)]
use ed25519_dalek::PublicKey;
use serde::Deserialize;
#[cfg(test)]
use std::fs;
#[cfg(test)]
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
        ton_block::ShardIdent::with_prefix_len(
            self.shardchain_pfx_len as u8,
            self.workchain,
            self.shardchain_pfx as u64,
        )
        .unwrap()
    }
}

/// Node config importer from JSON
#[derive(Deserialize)]
pub struct NodeApiConfig {
    #[serde(default = "NodeApiConfig::default_messages")]
    pub messages: String,
    #[serde(default = "NodeApiConfig::default_live_control")]
    pub live_control: String,
    #[serde(default = "NodeApiConfig::default_address")]
    pub address: String,
    #[serde(default = "NodeApiConfig::default_port")]
    pub port: u32,
}

impl Default for NodeApiConfig {
    fn default() -> Self {
        Self {
            messages: NodeApiConfig::default_messages(),
            live_control: NodeApiConfig::default_live_control(),
            address: NodeApiConfig::default_address(),
            port: NodeApiConfig::default_port(),
        }
    }
}

impl NodeApiConfig {
    fn default_messages() -> String {
        "topics/messages".to_string()
    }

    fn default_live_control() -> String {
        "se".to_string()
    }

    fn default_address() -> String {
        "127.0.0.1".to_string()
    }

    fn default_port() -> u32 {
        3000
    }
}

#[derive(Deserialize)]
pub struct NodeConfig {
    pub node_index: u8,
    pub port: u16,
    pub private_key: String,
    pub keys: Vec<String>,
    pub boot: Vec<String>,
    pub shard_id: ShardIdConfig,
    pub document_db: serde_json::Value,
    #[serde(default = "NodeConfig::default_log_path")]
    pub log_path: String,
    #[serde(default)]
    pub api: NodeApiConfig,
}

impl NodeConfig {
    fn default_log_path() -> String {
        "./log_cfg.yml".to_string()
    }
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// Import key values from list of files
    #[cfg(test)]
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

    pub fn document_db_config(&self) -> String {
        // If it has read to serde_json::Value it must be transformed to string
        serde_json::to_string(&self.document_db).unwrap()
    }
}
