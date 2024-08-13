use crate::data::KVStorage;
use crate::error::NodeResult;
use std::cmp::Ordering;
use std::sync::Mutex;
use ever_block::{Block, Serializable, ShardStateUnsplit, UInt256};

///
/// Hash of ShardState with block sequence number
///
#[derive(Clone, Debug, Default, Eq)]
pub struct ShardHash {
    pub block_seq_no: u64,
    pub shard_hash: UInt256,
}

impl ShardHash {
    /// Empty (start) shard hash
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            block_seq_no: 0,
            shard_hash: UInt256::from([0; 32]),
        }
    }
}

impl Ord for ShardHash {
    fn cmp(&self, other: &ShardHash) -> Ordering {
        self.block_seq_no.cmp(&other.block_seq_no)
    }
}

impl PartialOrd for ShardHash {
    fn partial_cmp(&self, other: &ShardHash) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for ShardHash {
    fn eq(&self, other: &ShardHash) -> bool {
        self.block_seq_no == other.block_seq_no
    }
}

///
/// Information about last block in shard
///
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShardStateInfo {
    /// Last block sequence number
    pub seq_no: u64,
    /// Last block end logical time
    pub lt: u64,
    /// Last block hash
    pub hash: UInt256,
}

impl ShardStateInfo {
    pub fn serialize(&self) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&(self.seq_no).to_be_bytes());
        data.extend_from_slice(&(self.lt).to_be_bytes());
        data.append(&mut self.hash.as_slice().to_vec());
        data
    }
}

pub struct ShardStorage {
    inner: Mutex<Box<dyn KVStorage + Send + Sync>>,
}

impl ShardStorage {
    pub(crate) fn new(inner: Box<dyn KVStorage + Send + Sync>) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }

    pub fn get(&self, key: &str) -> NodeResult<Vec<u8>> {
        self.inner.lock().unwrap().get(key)
    }

    pub(crate) fn set(&self, key: &str, data: Vec<u8>, overwrite: bool) -> NodeResult<()> {
        self.inner.lock().unwrap().set(key, data, overwrite)
    }

    pub(crate) fn save_non_finalized_block(&self, hash: UInt256, data: Vec<u8>) -> NodeResult<()> {
        self.set(
            &shard_storage_key::block_finality_hash_key(hash),
            data,
            false,
        )
    }

    pub(crate) fn save_serialized_shardstate_ex(
        &self,
        shard_state: &ShardStateUnsplit,
        shard_data: Option<Vec<u8>>,
        shard_hash: &UInt256,
        shard_state_info: ShardStateInfo,
    ) -> NodeResult<()> {
        assert_ne!(
            *shard_hash,
            UInt256::ZERO,
            "There should be no empty hashes!"
        );

        // save shard state
        let data = if let Some(shard_data) = shard_data {
            shard_data
        } else {
            shard_state.write_to_bytes()?
        };
        self.set(shard_storage_key::SHARD_STATE_BLOCK_KEY, data, true)?;
        self.set(
            shard_storage_key::SHARD_INFO_KEY,
            shard_state_info.serialize(),
            true,
        )
    }

    ///
    /// Save Shard State and block
    /// and shard.info and last_shard_hashes.info
    ///
    pub(crate) fn save_raw_block(
        &self,
        block: &Block,
        block_data: Option<&Vec<u8>>,
    ) -> NodeResult<()> {
        let info = block.read_info()?;
        let key = shard_storage_key::block_key(info.seq_no(), info.vert_seq_no());
        if let Some(block_data) = block_data {
            self.set(&key, block_data.clone(), true)?;
        } else {
            self.set(&key, block.write_to_bytes()?, true)?;
        }
        Ok(())
    }
}

pub mod shard_storage_key {
    use ever_block::UInt256;

    // ROOT
    pub const BLOCKS_FINALITY_INFO_KEY: &str = "blocks_finality.info";
    pub const SHARD_INFO_KEY: &str = "shard.info";
    pub const SHARD_STATE_BLOCK_KEY: &str = "shard_state.block";
    pub const ZEROSTATE_KEY: &str = "zerostate";

    // BLOCKS
    pub(crate) fn block_key(seq_no: u32, vert_seq_no: u32) -> String {
        format!("blocks/block_{:08X}_{:08X}.block", seq_no, vert_seq_no)
    }

    // BLOCK_FINALITY
    pub(crate) fn block_finality_hash_key(hash: UInt256) -> String {
        format!("block_finality/{}", hash.to_hex_string())
    }
}
