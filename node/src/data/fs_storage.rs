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

use crate::data::{KVStorage, NodeStorage};
use crate::error::{NodeError, NodeResult};
use std::clone::Clone;
use std::fs::{create_dir_all, File};
use std::io::prelude::*;
use std::path::PathBuf;
use ever_block::ShardIdent;

///
/// Shard Storage based on file system
/// It is supposed to be use for test and first launches node
///
pub struct FSStorage {
    root_path: PathBuf,
}

impl FSStorage {
    pub fn new(root_path: PathBuf) -> NodeResult<Self> {
        let root_path = root_path.join("workchains");
        if !root_path.as_path().exists() {
            create_dir_all(root_path.as_path())?;
        }
        Ok(Self { root_path })
    }

    pub fn clear_db(&self, shard: ShardIdent) -> NodeResult<()> {
        let mut storage = self.shard_storage(shard)?;
        storage.remove(super::shard_storage_key::BLOCKS_FINALITY_INFO_KEY)?;
        storage.remove(super::shard_storage_key::SHARD_INFO_KEY)?;
        storage.remove(super::shard_storage_key::SHARD_STATE_BLOCK_KEY)?;
        Ok(())
    }
}

impl NodeStorage for FSStorage {
    fn shard_storage(&self, shard: ShardIdent) -> NodeResult<Box<dyn KVStorage + Send + Sync>> {
        Ok(Box::new(FSKVStorage::with_path(
            shard.clone(),
            self.root_path.clone(),
        )?))
    }
}

pub struct FSKVStorage {
    root_path: PathBuf,
}

impl FSKVStorage {
    ///
    /// Create new instance of FileBasedStorage with custom root path
    /// Create catalog tree for storage
    /// root_path
    ///     workchains
    ///         MC | WC<id>
    ///             shard_(prefix)
    ///                 blocks
    ///                     block_(seq_no)_(ver_no)
    ///
    pub fn with_path(shard: ShardIdent, workchains_path: PathBuf) -> NodeResult<FSKVStorage> {
        let workchain_name = if shard.is_masterchain() {
            "MC".to_string()
        } else {
            format!("WC{}", shard.workchain_id())
        };
        let shard_name = format!("shard_{:016x}", shard.shard_prefix_with_tag());
        let root_path = workchains_path.join(workchain_name).join(shard_name);
        let blocks_dir = root_path.join("blocks");
        if !blocks_dir.exists() {
            create_dir_all(blocks_dir)?;
        }
        let block_finality_dir = root_path.join("block_finality");
        if !block_finality_dir.exists() {
            create_dir_all(block_finality_dir)?;
        }
        Ok(Self { root_path })
    }
}

impl KVStorage for FSKVStorage {
    fn get(&self, key: &str) -> NodeResult<Vec<u8>> {
        log::info!(target: "node", "load {}", key);
        let path = self.root_path.join(key);
        if !path.exists() {
            return Err(NodeError::NotFound);
        }
        let mut file = File::open(path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;
        Ok(data)
    }

    fn set(&mut self, key: &str, data: Vec<u8>, overwrite: bool) -> NodeResult<()> {
        log::info!(target: "node", "save {}", key);
        let path = self.root_path.join(key);
        if overwrite || !path.as_path().exists() {
            let mut file = File::create(path)?;
            file.write_all(&data)?;
            file.flush()?;
        }
        Ok(())
    }

    fn remove(&mut self, key: &str) -> NodeResult<()> {
        let path = self.root_path.join(key);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}
