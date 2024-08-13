use crate::data::{shard_storage_key, KVStorage, NodeStorage};
use crate::error::{NodeError, NodeResult};
use ever_block::ShardIdent;
use std::collections::HashMap;

pub struct MemStorage {
    zerostate: Vec<u8>,
}

impl MemStorage {
    pub fn new(zerostate: Vec<u8>) -> Self {
        Self { zerostate }
    }
}

impl NodeStorage for MemStorage {
    fn shard_storage(&self, shard: ShardIdent) -> NodeResult<Box<dyn KVStorage + Send + Sync>> {
        let mut storage = MemKVStorage(HashMap::new());
        if shard == ShardIdent::full(0) {
            let _ = storage.set(
                shard_storage_key::ZEROSTATE_KEY,
                self.zerostate.to_vec(),
                true,
            );
        }
        Ok(Box::new(storage))
    }
}

struct MemKVStorage(HashMap<String, Vec<u8>>);

impl KVStorage for MemKVStorage {
    fn get(&self, key: &str) -> NodeResult<Vec<u8>> {
        if let Some(data) = self.0.get(key) {
            Ok(data.clone())
        } else {
            Err(NodeError::NotFound)
        }
    }

    fn set(&mut self, key: &str, data: Vec<u8>, overwrite: bool) -> NodeResult<()> {
        if let Some(existing) = self.0.get_mut(key) {
            if overwrite {
                *existing = data;
            }
        } else {
            self.0.insert(key.to_string(), data);
        }
        Ok(())
    }

    fn remove(&mut self, key: &str) -> NodeResult<()> {
        let _ = self.0.remove(key);
        Ok(())
    }
}
