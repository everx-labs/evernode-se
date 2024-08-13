use crate::NodeResult;
use ever_block::ShardIdent;

mod arango;
mod documents_db_mock;
mod fork_provider;
mod fs_storage;
mod mem_documents_db;
mod mem_storage;
mod shard_storage;

pub use arango::ArangoHelper;
pub use fork_provider::ForkProvider;
pub use fs_storage::FSStorage;
pub use mem_storage::MemStorage;
pub use shard_storage::{shard_storage_key, ShardStateInfo, ShardStorage};

#[cfg(test)]
pub use fs_storage::FSKVStorage;

pub use mem_documents_db::MemDocumentsDb;

pub trait NodeStorage {
    fn shard_storage(&self, shard: ShardIdent) -> NodeResult<Box<dyn KVStorage + Send + Sync>>;
}

pub trait KVStorage {
    fn get(&self, key: &str) -> NodeResult<Vec<u8>>;
    fn set(&mut self, key: &str, data: Vec<u8>, overwrite: bool) -> NodeResult<()>;
    fn remove(&mut self, key: &str) -> NodeResult<()>;
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

pub trait ExternalAccountsProvider: Send + Sync {
    fn get_account(
        &self,
        address: ever_block::MsgAddressInt,
    ) -> NodeResult<Option<ever_block::ShardAccount>>;
}
