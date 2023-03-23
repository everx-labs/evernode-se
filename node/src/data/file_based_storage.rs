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

use super::{BlocksStorage, ShardStateStorage, TransactionsStorage};
use crate::data::{FinalityStorage, ShardStateInfo};
use crate::error::NodeResult;
use parking_lot::Mutex;
use std;
use std::clone::Clone;
use std::cmp::Ordering;
use std::convert::From;
use std::fs::{create_dir_all, File};
use std::io::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use ton_block::{
    Block, Deserializable, OutMsgQueueKey, Serializable, ShardIdent, ShardStateUnsplit, Transaction,
};
use ton_types::cells_serialization::{deserialize_tree_of_cells, serialize_tree_of_cells};
use ton_types::{types::UInt256, AccountId, Cell};

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
/// Shard Storage based on file system
/// It is supposed to be use for test and first launches node
///
#[allow(dead_code)]
pub struct FileBasedStorage {
    root_path: PathBuf,
    shards_path: PathBuf,
    shard_ident: ShardIdent,
    last_block: Arc<Mutex<Block>>,
    cache_to_write: Arc<Mutex<Vec<Block>>>,
}

impl FileBasedStorage {
    ///
    /// Create new instance of FileBasedStorage with custom root path
    ///
    pub fn with_path(shard_ident: ShardIdent, root_path: PathBuf) -> NodeResult<FileBasedStorage> {
        let shards_path = Self::create_workchains_dir(&root_path)?;
        Ok(FileBasedStorage {
            root_path,
            shards_path,
            shard_ident,
            last_block: Arc::new(Mutex::new(Block::default())),
            cache_to_write: Arc::new(Mutex::new(vec![])),
        })
    }

    /// Create "Shards" directory
    pub fn create_workchains_dir(root: &PathBuf) -> NodeResult<PathBuf> {
        let shards = root.join("workchains");
        if !shards.as_path().exists() {
            create_dir_all(shards.as_path())?;
        }
        Ok(shards)
    }

    ///
    /// Create catalog tree for storage
    /// root_path
    ///     workchains
    ///         MC | WC<id>
    ///             shard_(prefix)
    ///                 blocks
    ///                     block_(seq_no)_(ver_no)
    ///
    /// returned Shard_state_path and Blocks_dir_path
    pub fn create_default_shard_catalog(
        workchains_dir: PathBuf,
        shard_ident: &ShardIdent,
    ) -> NodeResult<(PathBuf, PathBuf, PathBuf)> {
        let workchain_name = if shard_ident.is_masterchain() {
            "MC".to_string()
        } else {
            format!("WC{}", shard_ident.workchain_id())
        };
        let shard_name = format!("shard_{:016x}", shard_ident.shard_prefix_with_tag());
        let shard_dir = workchains_dir.join(workchain_name).join(shard_name);
        let blocks_dir = shard_dir.join("blocks");
        let transactions_dir = shard_dir.join("transactions");
        if !blocks_dir.as_path().exists() {
            create_dir_all(blocks_dir.as_path())?;
        }
        if !transactions_dir.as_path().exists() {
            create_dir_all(transactions_dir.as_path())?;
        }
        Ok((shard_dir, blocks_dir, transactions_dir))
    }

    fn key_by_seqno(seq_no: u32, vert_seq_no: u32) -> u64 {
        ((vert_seq_no as u64) << 32) & (seq_no as u64)
    }

    fn int_save_block(shard_dir: PathBuf, block: &Block, root_hash: UInt256) -> NodeResult<()> {
        let ssi = ShardStateInfo::with_params(
            Self::key_by_seqno(
                block.read_info()?.seq_no(),
                block.read_info()?.vert_seq_no(),
            ),
            block.read_info()?.end_lt(),
            root_hash,
        );

        let (mut shard_path, mut blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &block.read_info()?.shard())?;
        shard_path.push("shard.info");

        let mut file_info = File::create(shard_path)?;
        file_info.write_all(ssi.serialize().as_slice())?;
        file_info.flush()?;

        blocks_path.push(format!(
            "block_{:08X}_{:08X}.block",
            block.read_info()?.seq_no(),
            block.read_info()?.vert_seq_no()
        ));

        block.write_to_file(blocks_path.as_path())?;

        Ok(())
    }
}

const BLOCK_FINALITY_FOLDER: &str = "block_finality";

impl FinalityStorage for FileBasedStorage {
    fn save_non_finalized_block(
        &self,
        hash: UInt256,
        _seq_no: u64,
        mut data: Vec<u8>,
    ) -> NodeResult<()> {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;

        let mut block_finality_path = shard_path.clone();
        block_finality_path.push(BLOCK_FINALITY_FOLDER);
        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }

        let mut name = block_finality_path.clone();
        name.push(hash.to_hex_string());
        log::info!(target: "node", "save finality block name: {:?}", name);
        if !name.as_path().exists() {
            let mut file_info = File::create(name)?;
            file_info.write_all(&mut data[..])?;
            file_info.flush()?;
        }
        Ok(())
    }

    fn load_non_finalized_block_by_seq_no(&self, _seq_no: u64) -> NodeResult<Vec<u8>> {
        unimplemented!()
    }

    fn load_non_finalized_block_by_hash(&self, hash: UInt256) -> NodeResult<Vec<u8>> {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;

        let mut block_finality_path = shard_path.clone();
        block_finality_path.push(BLOCK_FINALITY_FOLDER);
        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }

        let mut name = block_finality_path.clone();
        name.push(hash.to_hex_string());

        log::info!(target: "node", "load finality block name: {:?}", name);
        let mut file_info = File::open(name)?;
        let mut data = Vec::new();
        file_info.read_to_end(&mut data)?;
        Ok(data)
    }

    fn remove_form_finality_storage(&self, hash: UInt256) -> NodeResult<()> {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;
        let mut block_finality_path = shard_path.clone();

        block_finality_path.push(BLOCK_FINALITY_FOLDER);

        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }

        let mut name = block_finality_path.clone();

        name.push(hash.to_hex_string());
        if name.as_path().exists() {
            std::fs::remove_file(name)?;
        }
        Ok(())
    }

    fn save_custom_finality_info(&self, key: String, mut data: Vec<u8>) -> NodeResult<()> {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;

        if !shard_path.as_path().exists() {
            create_dir_all(shard_path.as_path())?;
        }

        let mut name = shard_path.clone();
        name.push(key);
        if !name.as_path().exists() {
            let mut file_info = File::create(name)?;
            file_info.write_all(&mut data[..])?;
            file_info.flush()?;
        }
        Ok(())
    }
    fn load_custom_finality_info(&self, key: String) -> NodeResult<Vec<u8>> {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;

        if !shard_path.as_path().exists() {
            create_dir_all(shard_path.as_path())?;
        }

        let mut name = shard_path.clone();
        name.push(key);
        let mut data = Vec::new();
        let mut file_info = File::open(name)?;
        file_info.read_to_end(&mut data)?;
        Ok(data)
    }
}

///
/// Implementation of ShardStateStorage for FileBasedStorage
///
impl ShardStateStorage for FileBasedStorage {
    ///
    /// Get selected shard state from file
    ///
    fn shard_state(&self) -> NodeResult<ShardStateUnsplit> {
        let cell = self.shard_bag()?;
        Ok(ShardStateUnsplit::construct_from_cell(cell)?)
    }

    fn shard_bag(&self) -> NodeResult<Cell> {
        let shard_dir = self.shards_path.clone();
        let (mut shard_path, _blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        shard_path.push("shard_state.block");

        let mut file = File::open(shard_path.as_path())?;
        // TODO: BOC from file
        Ok(deserialize_tree_of_cells(&mut file)?)
    }

    ///
    /// Save shard state to file
    ///
    fn save_shard_state(&self, shard_state: &ShardStateUnsplit) -> NodeResult<()> {
        let shard_dir = self.shards_path.clone();
        let (mut shard_path, _blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        shard_path.push("shard_state.block");

        let cell = shard_state.serialize()?;
        let mut file = File::create(shard_path.as_path())?;
        serialize_tree_of_cells(&cell, &mut file)?;
        file.flush()?;
        Ok(())
    }

    /// get serialized shard state
    fn serialized_shardstate(&self) -> NodeResult<Vec<u8>> {
        let shard_dir = self.shards_path.clone();
        let (mut shard_path, _blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        shard_path.push("shard_state.block");

        let mut file = File::open(shard_path.as_path())?;
        let mut buffer = vec![];
        file.read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    fn save_serialized_shardstate(&self, data: Vec<u8>) -> NodeResult<()> {
        let shard_dir = self.shards_path.clone();
        let (mut shard_path, _blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        shard_path.push("shard_state.block");

        let mut file = File::create(shard_path.as_path())?;
        file.write_all(data.as_slice())?;
        file.flush()?;
        Ok(())
    }

    ///
    /// Save Shard State
    /// and shard.info and last_shard_hashes.info
    ///
    fn save_serialized_shardstate_ex(
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

        let shard_dir = self.shards_path.clone();
        let (shard_path, _blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;

        // save shard state
        let mut shard_block = shard_path.clone();
        shard_block.push("shard_state.block");
        let mut file = File::create(shard_block.as_path())?;
        if shard_data.is_some() {
            file.write_all(shard_data.unwrap().as_slice())?;
        } else {
            let cell = shard_state.serialize()?;
            serialize_tree_of_cells(&cell, &mut file)?;
        }
        file.flush()?;

        // save shard info
        let mut shard_info = shard_path.clone();
        shard_info.push("shard.info");
        let mut file_info = File::create(shard_info)?;
        file_info.write_all(shard_state_info.serialize().as_slice())?;
        file_info.flush()?;

        Ok(())
    }
}

///
/// Implementation of BlocksStorage for FileBasedStorage
///
impl BlocksStorage for FileBasedStorage {
    ///
    /// Get selected block from shard storage
    ///
    fn block(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Block> {
        let shard_dir = self.shards_path.clone();
        let (_shard_path, mut blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        blocks_path.push(format!("block_{:08X}_{:08X}.block", seq_no, vert_seq_no));

        Ok(Block::construct_from_file(blocks_path.as_path())?)
    }

    fn raw_block(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Vec<u8>> {
        let shard_dir = self.shards_path.clone();
        let (_shard_path, mut blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        blocks_path.push(format!("block_{:08X}_{:08X}.block", seq_no, vert_seq_no));

        let mut file = File::open(blocks_path.as_path())?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)?;
        Ok(buf)
    }

    ///
    /// Save block to file based shard storage
    ///
    fn save_block(&self, block: &Block, root_hash: UInt256) -> NodeResult<()> {
        let mut last_block = self.last_block.lock();
        *last_block = block.clone();
        let shard_dir = self.shards_path.clone();

        Self::int_save_block(shard_dir, block, root_hash)?;

        Ok(())
    }

    ///
    /// Save Shard State and block
    /// and shard.info and last_shard_hashes.info
    ///
    fn save_raw_block(&self, block: &Block, block_data: Option<&Vec<u8>>) -> NodeResult<()> {
        let shard_dir = self.shards_path.clone();
        let (_shard_path, mut blocks_path, _tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;

        let block = block.clone(); // temp
                                   // save block
        blocks_path.push(format!(
            "block_{:08X}_{:08X}.block",
            block.read_info()?.seq_no(),
            block.read_info()?.vert_seq_no()
        ));

        if let Some(block_data) = block_data {
            let mut file = File::create(blocks_path.as_path()).unwrap();
            file.write_all(block_data)
                .expect("Error write signed block data to file.");
            file.flush().unwrap();
        } else {
            block
                .write_to_file(blocks_path.as_path())
                .expect("Error write signed block to file.");
        }

        Ok(())
    }
}

///
/// Implementation of TransactionsStorage for FileBasedStorage
///
impl TransactionsStorage for FileBasedStorage {
    fn save_transaction(&self, tr: Arc<Transaction>) -> NodeResult<()> {
        let shard_dir = self.shards_path.clone();
        let (_shard_path, _blocks_path, mut tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        tr_dir.push(format!(
            "tr_{}_{:08X}.boc",
            tr.logical_time(),
            OutMsgQueueKey::first_u64(tr.account_id())
        ));
        // tr.write_to_file(tr_dir.to_str().unwrap())?;
        Ok(())
    }
    fn find_by_lt(&self, lt: u64, acc_id: &AccountId) -> NodeResult<Option<Transaction>> {
        let shard_dir = self.shards_path.clone();
        let (_shard_path, _blocks_path, mut tr_dir) =
            Self::create_default_shard_catalog(shard_dir, &self.shard_ident)?;
        tr_dir.push(format!(
            "tr_{}_{:08X}.boc",
            lt,
            OutMsgQueueKey::first_u64(acc_id)
        ));
        if let Ok(bytes) = std::fs::read(tr_dir.to_str().unwrap()) {
            return Ok(Some(Transaction::construct_from_bytes(&bytes)?));
        }
        Ok(None)
    }
}

//     fn save_serialized_shardstate_ex(&self, shard_state: &ShardStateUnsplit,
//             shard_data: Option<Vec<u8>>, shard_hash: &UInt256,
//             shard_state_info: ShardStateInfo) -> NodeResult<()>;
// }
