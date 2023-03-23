use crate::NodeResult;
#[cfg(test)]
use std::io::Read;
use std::sync::Arc;
use ton_block::{Block, ShardStateUnsplit, Transaction};
#[cfg(test)]
use ton_types::ByteOrderRead;
use ton_types::{AccountId, Cell, UInt256};

mod arango;
mod documents_db_mock;
mod file_based_storage;
mod mem_documents_db;
#[cfg(test)]
mod test_storage;

#[cfg(test)]
pub use test_storage::TestStorage;

pub use arango::ArangoHelper;

#[cfg(test)]
pub use documents_db_mock::DocumentsDbMock;

pub use file_based_storage::FileBasedStorage;
pub use mem_documents_db::MemDocumentsDb;
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
    pub fn with_params(seq_no: u64, lt: u64, hash: UInt256) -> Self {
        Self { seq_no, lt, hash }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&(self.seq_no).to_be_bytes());
        data.extend_from_slice(&(self.lt).to_be_bytes());
        data.append(&mut self.hash.as_slice().to_vec());
        data
    }

    #[cfg(test)]
    pub fn deserialize<R: Read>(rdr: &mut R) -> NodeResult<Self> {
        let seq_no = rdr.read_be_u64()?;
        let lt = rdr.read_be_u64()?;
        let hash = UInt256::from(rdr.read_u256()?);
        Ok(ShardStateInfo { seq_no, lt, hash })
    }
}

/// Trait for shard state storage
pub trait ShardStateStorage {
    fn shard_state(&self) -> NodeResult<ShardStateUnsplit>;
    fn shard_bag(&self) -> NodeResult<Cell>;
    fn save_shard_state(&self, shard_state: &ShardStateUnsplit) -> NodeResult<()>;
    fn serialized_shardstate(&self) -> NodeResult<Vec<u8>>;
    fn save_serialized_shardstate(&self, data: Vec<u8>) -> NodeResult<()>;
    fn save_serialized_shardstate_ex(
        &self,
        shard_state: &ShardStateUnsplit,
        shard_data: Option<Vec<u8>>,
        shard_hash: &UInt256,
        shard_state_info: ShardStateInfo,
    ) -> NodeResult<()>;
}

// Trait for blocks storage (key-value)
pub trait BlocksStorage {
    fn block(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Block>;
    fn raw_block(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Vec<u8>>;
    fn save_block(&self, block: &Block, root_hash: UInt256) -> NodeResult<()>;
    fn save_raw_block(&self, block: &Block, block_data: Option<&Vec<u8>>) -> NodeResult<()>;
}

/// Trait for transactions storage (this storage have to support difficult queries)
pub trait TransactionsStorage {
    fn save_transaction(&self, tr: Arc<Transaction>) -> NodeResult<()>;
    fn find_by_lt(&self, _lt: u64, _acc_id: &AccountId) -> NodeResult<Option<Transaction>> {
        unimplemented!()
    }
}

pub struct SerializedItem {
    pub id: String,
    pub data: serde_json::Value,
}

pub trait DocumentsDb: Send + Sync {
    fn put_account(&self, item: SerializedItem) -> NodeResult<()>;
    fn put_block(&self, item: SerializedItem) -> NodeResult<()>;
    fn put_message(&self, item: SerializedItem) -> NodeResult<()>;
    fn put_transaction(&self, item: SerializedItem) -> NodeResult<()>;
    fn has_delivery_problems(&self) -> bool;
}

/// Trait for finality storage
pub trait FinalityStorage {
    fn save_non_finalized_block(&self, hash: UInt256, seq_no: u64, data: Vec<u8>)
        -> NodeResult<()>;
    fn load_non_finalized_block_by_seq_no(&self, seq_no: u64) -> NodeResult<Vec<u8>>;
    fn load_non_finalized_block_by_hash(&self, hash: UInt256) -> NodeResult<Vec<u8>>;
    fn remove_form_finality_storage(&self, hash: UInt256) -> NodeResult<()>;

    fn save_custom_finality_info(&self, key: String, data: Vec<u8>) -> NodeResult<()>;
    fn load_custom_finality_info(&self, key: String) -> NodeResult<Vec<u8>>;
}
