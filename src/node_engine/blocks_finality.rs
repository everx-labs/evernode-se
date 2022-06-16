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

use super::*;
use crate::error::NodeError;
use rand::Rng;
use std::{
    collections::HashSet,
    fs::{create_dir_all, File},
    io::{ErrorKind, Read, Seek, Write},
    path::PathBuf,
    time::Duration,
};
use ton_block::{
    Account, AccountStatus, BlkPrevInfo, Block, CurrencyCollection, Deserializable, ExtBlkRef,
    ExtOutMessageHeader, ExternalInboundMessageHeader, GetRepresentationHash, HashmapAugType,
    InMsg, InternalMessageHeader, Message, MsgAddressExt, MsgAddressInt, MsgEnvelope, OutMsg,
    Serializable, ShardIdent, ShardStateUnsplit, Transaction, UnixTime32,
};
use ton_types::{deserialize_tree_of_cells, serialize_toc, SliceData, UInt256};

#[cfg(test)]
#[path = "../../tonos-se-tests/unit/test_block_finality.rs"]
mod tests;

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

pub trait DocumentsDb: Send + Sync {
    fn put_account(&self, acc: Account) -> NodeResult<()>;
    fn put_deleted_account(&self, workchain_id: i32, account_id: AccountId) -> NodeResult<()>;
    fn put_block(&self, block: &Block) -> NodeResult<()>;

    fn put_message(
        &self,
        msg: Message,
        transaction_id: Option<UInt256>,
        transaction_now: Option<u32>,
        block_id: Option<UInt256>,
    ) -> NodeResult<()>;

    fn put_transaction(
        &self,
        tr: Transaction,
        block_id: Option<UInt256>,
        workchain_id: i32,
    ) -> NodeResult<()>;

    fn has_delivery_problems(&self) -> bool;
}

/// Structure for Block finality layer
/// This realize next functionality:
/// Store block received from POA
/// Finalize block witch POA finality - store to permanent storage
/// Store (in memory) current shard-state for every block-candidate
/// Get current shard-state
/// Get last block by number or hash
/// Rollback block (and shardstate) if block is rolled
#[allow(dead_code)]
pub struct OrdinaryBlockFinality<S, B, T, F>
where
    S: ShardStateStorage,
    B: BlocksStorage,
    T: TransactionsStorage,
    F: FinalityStorage,
{
    shard_ident: ShardIdent,
    root_path: PathBuf,
    shard_state_storage: Arc<S>,
    blocks_storage: Arc<B>,
    tr_storage: Arc<T>,
    fn_storage: Arc<F>,
    db: Option<Arc<dyn DocumentsDb>>,

    current_block: Box<ShardBlock>,
    blocks_by_hash: HashMap<UInt256, Box<FinalityBlock>>, // need to remove
    blocks_by_no: HashMap<u64, Box<FinalityBlock>>, // need to remove
    last_finalized_block: Box<ShardBlock>,
}

fn key_by_seqno(seq_no: u32, vert_seq_no: u32) -> u64 {
    ((vert_seq_no as u64) << 32) | (seq_no as u64)
}

impl<S, B, T, F> OrdinaryBlockFinality<S, B, T, F>
where
    S: ShardStateStorage,
    B: BlocksStorage,
    T: TransactionsStorage,
    F: FinalityStorage,
{
    /// Create new instance BlockFinality
    /// with all kind of storages
    pub fn with_params(
        shard_ident: ShardIdent,
        root_path: PathBuf,
        shard_state_storage: Arc<S>,
        blocks_storage: Arc<B>,
        tr_storage: Arc<T>,
        fn_storage: Arc<F>,
        db: Option<Arc<dyn DocumentsDb>>,
        _public_keys: Vec<ed25519_dalek::PublicKey>,
    ) -> Self {
        let root_path = FileBasedStorage::create_workchains_dir(&root_path)
            .expect("cannot create shards directory");

        OrdinaryBlockFinality {
            shard_ident,
            root_path,
            shard_state_storage,
            blocks_storage,
            tr_storage,
            fn_storage,
            db,
            current_block: Box::new(ShardBlock::default()),
            blocks_by_hash: HashMap::new(),
            blocks_by_no: HashMap::new(),
            last_finalized_block: Box::new(ShardBlock::default()),
        }
    }

    fn finality_blocks(&mut self, root_hash: UInt256) -> NodeResult<()> {
        log::debug!("FIN-BLK {:x}", root_hash);
        if let Some(fin_sb) = self.blocks_by_hash.remove(&root_hash) {
            let sb = self.stored_to_loaded(fin_sb)?;

            // create shard state info
            let info = sb.block.read_info()?;
            let shard_info = ShardStateInfo {
                seq_no: key_by_seqno(info.seq_no(), info.vert_seq_no()),
                lt: info.end_lt(),
                hash: sb.root_hash.clone(),
            };
            let shard = sb.shard_state.write_to_bytes()?;
            // save shard state
            self.shard_state_storage.save_serialized_shardstate_ex(
                &ShardStateUnsplit::default(),
                Some(shard),
                &sb.block.read_state_update()?.new_hash,
                shard_info,
            )?;
            // save finalized block
            self.blocks_storage
                .save_raw_block(&sb.block, Some(&sb.serialized_block))?;
            // remove shard-block from number hashmap
            self.blocks_by_no
                .remove(&sb.seq_no)
                .expect("block by number remove error");

            if let Err(err) = self.reflect_block_in_db(&sb.block, sb.shard_state.clone()) {
                log::warn!(target: "node", "reflect_block_in_db(Finalized) error: {}", err);
            }

            self.last_finalized_block = sb;
            log::info!(target: "node", "FINALITY:save block seq_no: {:?}, serialized len = {}",
                self.last_finalized_block.block.read_info()?.seq_no(),
                self.last_finalized_block.serialized_block.len()
            );
        } else {
            if root_hash != UInt256::ZERO && root_hash != self.last_finalized_block.root_hash {
                log::warn!(target: "node", "Can`t finality unknown hash!!!");
                return Err(NodeError::FinalityError);
            }
        }
        Ok(())
    }

    fn stored_to_loaded(&self, fin_sb: Box<FinalityBlock>) -> NodeResult<Box<ShardBlock>> {
        Ok(match *fin_sb {
            FinalityBlock::Stored(sb_hash) => {
                // load sb
                let (shard_path, _blocks_path, _tr_dir) =
                    FileBasedStorage::create_default_shard_catalog(
                        self.root_path.clone(),
                        &self.shard_ident,
                    )?;
                let mut block_sb_path = shard_path.clone();
                block_sb_path.push("block_finality");
                if !block_sb_path.as_path().exists() {
                    create_dir_all(block_sb_path.as_path())?;
                }

                block_sb_path.push(sb_hash.root_hash.to_hex_string());

                self.read_one_sb_from_file(block_sb_path)?
            }
            FinalityBlock::Loaded(l_sb) => l_sb,
        })
    }

    fn save_one(sb: Box<ShardBlock>, file_name: PathBuf) -> NodeResult<()> {
        let mut file_info = File::create(file_name)?;
        let mut data = sb.serialize()?;
        file_info.write_all(&mut data[..])?;
        file_info.flush()?;
        Ok(())
    }

    pub fn save_finality(&self) -> NodeResult<()> {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;
        let mut block_finality_path = shard_path.clone();
        block_finality_path.push("block_finality");
        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }
        let mut name = block_finality_path.clone();
        name.push(self.current_block.root_hash.to_hex_string());
        if !name.as_path().exists() {
            Self::save_one(self.current_block.clone(), name)?;
        }
        let mut name = block_finality_path.clone();
        name.push(self.last_finalized_block.root_hash.to_hex_string());
        if !name.as_path().exists() {
            Self::save_one(self.last_finalized_block.clone(), name)?;
        }
        self.save_index()?;
        Ok(())
    }

    fn save_index(&self) -> NodeResult<()> {
        let (mut shard_path, _blocks_path, _tr_dir) =
            FileBasedStorage::create_default_shard_catalog(
                self.root_path.clone(),
                &self.shard_ident,
            )?;
        shard_path.push("blocks_finality.info");
        let mut file_info = File::create(shard_path)?;
        self.serialize(&mut file_info)?;
        file_info.flush()?;
        Ok(())
    }

    ///
    /// Load from disk
    ///
    pub fn load(&mut self) -> NodeResult<()> {
        let (mut shard_path, _blocks_path, _tr_dir) =
            FileBasedStorage::create_default_shard_catalog(
                self.root_path.clone(),
                &self.shard_ident,
            )?;
        shard_path.push("blocks_finality.info");
        log::info!(target: "node", "load: {}", shard_path.to_str().unwrap());
        let mut file_info = File::open(shard_path)?;
        self.deserialize(&mut file_info)?;
        Ok(())
    }

    ///
    /// Write data BlockFinality to file
    ///
    fn serialize(&self, writer: &mut dyn Write) -> NodeResult<()> {
        // serialize struct:
        // 32bit - length of structure ShardBlock
        // structure ShardBlock
        // ...
        // first save current block
        writer.write_all(self.current_block.root_hash.as_slice())?;
        writer.write_all(&self.current_block.seq_no.to_le_bytes())?;
        // save last finality block
        writer.write_all(self.last_finalized_block.root_hash.as_slice())?;
        writer.write_all(&self.last_finalized_block.seq_no.to_le_bytes())?;
        for (key, sb) in self.blocks_by_hash.iter() {
            writer.write_all(key.as_slice())?;
            writer.write_all(&sb.seq_no().to_le_bytes())?;
        }
        Ok(())
    }

    fn read_one_sb_hash<R>(&self, rdr: &mut R) -> NodeResult<(u64, UInt256)>
    where
        R: Read + Seek,
    {
        // first read current block
        let hash = rdr.read_u256()?;
        let seq_no = rdr.read_le_u64()?;
        Ok((seq_no, UInt256::from(hash)))
    }

    fn read_one_sb<R>(&self, rdr: &mut R) -> NodeResult<Box<ShardBlock>>
    where
        R: Read + Seek,
    {
        let (shard_path, _blocks_path, _tr_dir) = FileBasedStorage::create_default_shard_catalog(
            self.root_path.clone(),
            &self.shard_ident,
        )?;
        let mut block_finality_path = shard_path.clone();
        block_finality_path.push("block_finality");
        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }
        // first read current block
        let hash = UInt256::from(rdr.read_u256()?);
        let seq_no = rdr.read_le_u64()?;
        log::info!(target: "node", "read_one_sb:seq_no: {}", seq_no);
        if seq_no == 0 {
            Ok(Box::new(ShardBlock::default()))
        } else {
            let mut current_block_name = block_finality_path.clone();
            current_block_name.push(hash.to_hex_string());

            self.read_one_sb_from_file(current_block_name)
        }
    }

    fn read_one_sb_from_file(&self, file_name: PathBuf) -> NodeResult<Box<ShardBlock>> {
        log::info!(target: "node", "load {}", file_name.to_str().unwrap());
        let mut file_info = File::open(file_name.clone())?;
        let mut data = Vec::new();
        file_info.read_to_end(&mut data)?;
        log::info!(target: "node", "load {} ok.", file_name.to_str().unwrap());
        Ok(Box::new(ShardBlock::deserialize(
            &mut std::io::Cursor::new(data),
        )?))
    }

    ///
    /// Read saved data BlockFinality from file
    ///
    pub fn deserialize<R>(&mut self, rdr: &mut R) -> NodeResult<()>
    where
        R: Read + Seek,
    {
        log::info!(target: "node", "load current block");
        self.current_block = self.read_one_sb(rdr)?;
        log::info!(target: "node", "load last finalized block");
        self.last_finalized_block = self.read_one_sb(rdr)?;
        loop {
            log::info!(target: "node", "load non finalized block");
            match self.read_one_sb_hash(rdr) {
                Ok((seq_no, hash)) => {
                    let sb_hash = Box::new(FinalityBlock::Stored(Box::new(
                        ShardBlockHash::with_hash(seq_no.clone(), hash.clone()),
                    )));
                    self.blocks_by_hash.insert(hash.clone(), sb_hash.clone());
                    self.blocks_by_no.insert(seq_no.clone(), sb_hash.clone());
                }
                Err(NodeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// save objects into kafka with "finalized" state,
    fn reflect_block_in_db(
        &self,
        block: &Block,
        shard_state: Arc<ShardStateUnsplit>,
    ) -> NodeResult<()> {
        if let Some(db) = self.db.clone() {
            // let block_root = block.serialize()?;
            // let state_root = shard_state.serialize()?;
            // let block_info_root = block.read_info()?.serialize()?;
            // let block_info_cells = BagOfCells::with_root(&block_info_root.into())
            //     .withdraw_cells();

            let block_id = block.hash()?;
            let extra = block.read_extra()?;
            let workchain_id = block.read_info()?.shard().workchain_id();

            extra.read_in_msg_descr()?.iterate_objects(|in_msg| {
                let msg = in_msg.read_message()?;
                log::debug!(target: "node", "PUT-IN-MESSAGE-BLOCK {:x}", msg.hash().unwrap());
                // msg.prepare_proof_for_json(&block_info_cells, &block_root)?;
                // msg.prepare_boc_for_json()?;
                let transaction_id = in_msg.transaction_cell().map(|cell| cell.repr_hash());
                let transaction_now = in_msg
                    .read_transaction()?
                    .map(|transaction| transaction.now());
                db.put_message(msg, transaction_id, transaction_now, Some(block_id.clone()))
                    .map_err(
                        |err| log::warn!(target: "node", "reflect message to DB(1). error: {}", err),
                    )
                    .ok();
                Ok(true)
            })?;

            log::debug!(target: "node", "in_msg_descr.iterate - success");

            extra.read_out_msg_descr()?.iterate_objects(|out_msg| {
                let msg = out_msg.read_message()?.unwrap();
                log::debug!(target: "node", "PUT-OUT-MESSAGE-BLOCK {:?}", msg);
                // msg1.prepare_proof_for_json(&block_info_cells, &block_root)?;
                // msg1.prepare_boc_for_json()?;
                let transaction_id = out_msg.transaction_cell().map(|cell| cell.repr_hash());
                db.put_message(msg, transaction_id, None, Some(block_id.clone()))
                    .map_err(
                        |err| log::warn!(target: "node", "reflect message to DB(2). error: {}", err),
                    )
                    .ok();
                Ok(true)
            })?;

            log::debug!(target: "node", "out_msg_descr.iterate - success");

            let mut changed_acc = HashSet::new();

            extra.read_account_blocks()?.iterate_with_keys(|account_id, account_block| {
                let mut orig_status = None;
                let mut end_status = None;
                changed_acc.insert(account_id);
                account_block.transaction_iterate(|transaction| {
                    // transaction.prepare_proof_for_json(&block_info_cells, &block_root)?;
                    // transaction.prepare_boc_for_json()?;
log::debug!(target: "node", "PUT-TRANSACTION-BLOCK {:x}", transaction.hash()?);
                    if orig_status.is_none() {
                        orig_status = Some(transaction.orig_status.clone());
                    }
                    end_status = Some(transaction.end_status.clone());
                    if let Err(err) = db.put_transaction(transaction, Some(block_id.clone()), workchain_id) {
                        log::warn!(target: "node", "reflect transaction to DB. error: {}", err);
                    }
                    Ok(true)
                })?;
                if end_status == Some(AccountStatus::AccStateNonexist) &&
                    end_status != orig_status
                {
                    let account_id = account_block.account_id().clone();
                    if let Err(err) = db.put_deleted_account(workchain_id, account_id) {
                        log::warn!(target: "node", "reflect deleted account to DB. error: {}", err);
                    }
                }
                Ok(true)
            })?;

            //if block_status == BlockProcessingStatus::Finalized {
            shard_state.read_accounts()?.iterate_with_keys(|id, acc| {
                if changed_acc.contains(&id) {
                    // acc.account.prepare_proof_for_json(&state_root)?;
                    // acc.account.prepare_boc_for_json()?;
                    let acc = acc.read_account()?;
                    if acc.is_none() {
                        log::error!(target: "node", "something gone wrong with account {:x}", id);
                    } else if let Err(err) = db.put_account(acc) {
                        log::warn!(target: "node", "reflect account to DB. error: {}", err);
                    }
                }
                Ok(true)
            })?;
            //}

            log::debug!(target: "node", "accounts.iterate - success");

            db.put_block(block)?;
        }
        Ok(())
    }
}

impl<S, B, T, F> BlockFinality for OrdinaryBlockFinality<S, B, T, F>
where
    S: ShardStateStorage,
    B: BlocksStorage,
    T: TransactionsStorage,
    F: FinalityStorage,
{
    /// Save block until finality comes
    fn put_block_with_info(
        &mut self,
        block: &Block,
        shard_state: Arc<ShardStateUnsplit>,
    ) -> NodeResult<()> {
        log::info!(target: "node", "FINALITY:    block seq_no: {:?}", block.read_info()?.seq_no());

        let sb = Box::new(ShardBlock::with_block_and_state(block.clone(), shard_state));
        log::debug!(target: "node", "PUT-BLOCK-HASH {:?}", sb.root_hash);

        self.current_block = sb.clone();

        // insert block to map
        let sb_hash = Box::new(FinalityBlock::Loaded(sb.clone()));
        self.blocks_by_hash
            .insert(sb.root_hash.clone(), sb_hash.clone());
        self.blocks_by_no.insert(sb.get_seq_no(), sb_hash.clone());
        // finalize block by finality_hash
        self.finality_blocks(sb.root_hash.clone())?;
        self.save_finality()?;
        Ok(())
    }

    /// get last block sequence number
    fn get_last_seq_no(&self) -> u32 {
        self.current_block.block.read_info().unwrap().seq_no()
    }

    /// get last block info
    fn get_last_block_info(&self) -> NodeResult<BlkPrevInfo> {
        let info = &self.current_block.block.read_info()?;
        Ok(BlkPrevInfo::Block {
            prev: ExtBlkRef {
                end_lt: info.end_lt(),
                seq_no: info.seq_no(),
                root_hash: self.current_block.root_hash.clone(), //UInt256::from(self.current_block.block.root_hash().clone()),
                file_hash: self.current_block.file_hash.clone(),
            },
        })
    }

    /// get last shard bag
    fn get_last_shard_state(&self) -> Arc<ShardStateUnsplit> {
        //log::warn!("LAST SHARD BAG {}", self.current_block.shard_bag.get_repr_hash_by_index(0).unwrap().to_hex_string()));
        Arc::clone(&self.current_block.shard_state)
    }
    /// find block by hash and return his sequence number (for sync)
    fn find_block_by_hash(&self, hash: &UInt256) -> u64 {
        if self.blocks_by_hash.contains_key(hash) {
            self.blocks_by_hash.get(hash).unwrap().seq_no()
        } else {
            0xFFFFFFFF // if not found
        }
    }

    /// Rollback shard state to one of block candidates
    fn rollback_to(&mut self, hash: &UInt256) -> NodeResult<()> {
        if self.blocks_by_hash.contains_key(hash) {
            let sb = self.blocks_by_hash.remove(hash).unwrap();
            let mut seq_no = sb.seq_no();
            self.current_block = self.stored_to_loaded(sb)?;

            // remove from hashmaps all blocks with greater seq_no
            loop {
                let tmp = self.blocks_by_no.remove(&seq_no);
                if tmp.is_some() {
                    self.blocks_by_hash.remove(tmp.unwrap().root_hash());
                } else {
                    break;
                }
                seq_no += 1;
            }

            Ok(())
        } else {
            Err(NodeError::RoolbackBlockError)
        }
    }

    /// get raw signed block data - for synchronize
    fn get_raw_block_by_seqno(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Vec<u8>> {
        let key = key_by_seqno(seq_no, vert_seq_no);
        if self.blocks_by_no.contains_key(&key) {
            /* TODO: which case to use?
            return Ok(self.blocks_by_no.get(&key).unwrap().serialized_block.clone())
            TODO rewrite to
            return Ok(
                self.fn_storage.load_non_finalized_block_by_seq_no(key)?.serialized_block.clone()
            )*/
            return Ok(self
                .stored_to_loaded(self.blocks_by_no.get(&key).unwrap().clone())?
                .serialized_block
                .clone());
        }
        self.blocks_storage.raw_block(seq_no, vert_seq_no)
    }

    /// get number of last finalized shard
    fn get_last_finality_shard_hash(&self) -> NodeResult<(u64, UInt256)> {
        // TODO avoid serialization there
        let cell = self.last_finalized_block.shard_state.serialize()?;

        Ok((self.last_finalized_block.seq_no, cell.repr_hash()))
    }

    /// reset block finality
    /// clean all maps, load last finalized data
    fn reset(&mut self) -> NodeResult<()> {
        self.current_block = self.last_finalized_block.clone();
        // remove files from disk
        self.blocks_by_hash.clear();
        self.blocks_by_no.clear();
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FinalityBlock {
    Loaded(Box<ShardBlock>),
    Stored(Box<ShardBlockHash>),
}

impl FinalityBlock {
    pub fn seq_no(&self) -> u64 {
        match self {
            FinalityBlock::Stored(sb) => sb.seq_no,
            FinalityBlock::Loaded(sb) => sb.seq_no,
        }
    }

    pub fn root_hash(&self) -> &UInt256 {
        match self {
            FinalityBlock::Stored(sb) => &sb.root_hash,
            FinalityBlock::Loaded(sb) => &sb.root_hash,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShardBlockHash {
    seq_no: u64,
    root_hash: UInt256,
}

impl ShardBlockHash {
    pub fn with_hash(seq_no: u64, hash: UInt256) -> Self {
        Self {
            seq_no,
            root_hash: hash,
        }
    }
}

/// Structure for store one block and his ShardState
#[derive(Clone, Debug, PartialEq)]
pub struct ShardBlock {
    seq_no: u64,
    serialized_block: Vec<u8>,
    root_hash: UInt256,
    file_hash: UInt256,
    block: Block,
    shard_state: Arc<ShardStateUnsplit>,
}

impl Default for ShardBlock {
    fn default() -> Self {
        Self {
            seq_no: 0,
            serialized_block: Vec::new(),
            root_hash: UInt256::ZERO,
            file_hash: UInt256::ZERO,
            block: Block::default(),
            shard_state: Arc::new(ShardStateUnsplit::default()),
        }
    }
}

impl ShardBlock {
    /// Create new instance of ShardBlock
    pub fn new() -> Self {
        Self::default()
    }

    /// get current block sequence number
    pub fn get_seq_no(&self) -> u64 {
        self.seq_no
    }

    /// get current block hash
    pub fn root_hash(&self) -> &UInt256 {
        &self.root_hash
    }

    /// Create new instance of shard block with Block and new shard state
    pub fn with_block_and_state(block: Block, shard_state: Arc<ShardStateUnsplit>) -> Self {
        let cell = block.serialize().unwrap();
        let root_hash = cell.repr_hash();

        let serialized_block = serialize_toc(&cell).unwrap();
        let file_hash = UInt256::calc_file_hash(&serialized_block);
        let info = block.read_info().unwrap();

        Self {
            seq_no: key_by_seqno(info.seq_no(), info.vert_seq_no()),
            serialized_block,
            root_hash,
            file_hash,
            block,
            shard_state,
        }
    }

    /// serialize shard block (for save on disk)
    pub fn serialize(&self) -> NodeResult<Vec<u8>> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.seq_no.to_le_bytes());
        buf.extend_from_slice(&(self.serialized_block.len() as u32).to_le_bytes());
        buf.extend_from_slice(self.serialized_block.as_slice());
        buf.extend_from_slice(self.root_hash.as_slice());
        buf.extend_from_slice(self.file_hash.as_slice());

        buf.append(&mut self.shard_state.write_to_bytes()?);

        let mut block_buf = self.block.write_to_bytes()?;
        buf.append(&mut block_buf);
        Ok(buf)
    }

    /// deserialize shard block
    pub fn deserialize<R: Read + Seek>(rdr: &mut R) -> NodeResult<Self> {
        let mut sb = ShardBlock::new();
        sb.seq_no = rdr.read_le_u64()?;
        let sb_len = rdr.read_le_u32()?;
        let mut sb_buf = vec![0; sb_len as usize];
        rdr.read(&mut sb_buf)?;
        sb.serialized_block = sb_buf;

        let hash = rdr.read_u256()?;
        sb.root_hash = UInt256::from(hash);

        let hash = rdr.read_u256()?;
        sb.file_hash = UInt256::from(hash);

        let mut shard_slice = deserialize_tree_of_cells(rdr)?.into();
        sb.shard_state.read_from(&mut shard_slice)?;

        let cell = deserialize_tree_of_cells(rdr)?;
        sb.block = Block::construct_from_cell(cell)?;

        Ok(sb)
    }
}

// runs 10 thread to generate 5000 accounts with 1 input and two output messages per every block
// finalizes block and return
#[allow(dead_code)]
pub(crate) fn generate_block_with_seq_no(
    shard_ident: ShardIdent,
    seq_no: u32,
    prev_info: BlkPrevInfo,
) -> Block {
    let block_builder = Arc::new(BlockBuilder::with_shard_ident(
        shard_ident,
        seq_no,
        prev_info,
        0,
        None,
        UnixTime32::now().as_u32(),
    ));

    // start 10 thread for generate transaction
    for _ in 0..10 {
        let builder_clone = block_builder.clone();
        thread::spawn(move || {
            //println!("Thread write start.");
            let mut rng = rand::thread_rng();
            for _ in 0..5000 {
                let acc = AccountId::from_raw(
                    (0..32).map(|_| rand::random::<u8>()).collect::<Vec<u8>>(),
                    256,
                );
                let mut transaction = Transaction::with_address_and_status(
                    acc.clone(),
                    AccountStatus::AccStateActive,
                );

                let mut value = CurrencyCollection::default();
                value.grams = 10202u64.into();
                let mut imh = InternalMessageHeader::with_addresses(
                    MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    MsgAddressInt::with_standart(
                        None,
                        0,
                        AccountId::from_raw(
                            (0..32).map(|_| rand::random::<u8>()).collect::<Vec<u8>>(),
                            256,
                        ),
                    )
                    .unwrap(),
                    value,
                );

                imh.ihr_fee = 10u64.into();
                imh.fwd_fee = 5u64.into();
                let mut inmsg1 = Arc::new(Message::with_int_header(imh));
                if let Some(m) = Arc::get_mut(&mut inmsg1) {
                    m.set_body(SliceData::new(vec![0x21; 120]));
                }

                let env = MsgEnvelope::with_message_and_fee(&inmsg1, 9u64.into()).unwrap();
                let in_msg_int = InMsg::immediatelly_msg(
                    env.serialize().unwrap(),
                    transaction.serialize().unwrap(),
                    11u64.into(),
                );

                let ext_in_header = ExternalInboundMessageHeader {
                    src: MsgAddressExt::with_extern(SliceData::new(vec![
                        0x23, 0x52, 0x73, 0x00, 0x80,
                    ]))
                    .unwrap(),
                    dst: MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    import_fee: 10u64.into(),
                };

                let mut inmsg = Message::with_ext_in_header(ext_in_header);
                inmsg.set_body(SliceData::new(vec![0x01; 120]));

                transaction.write_in_msg(Some(&inmsg1)).unwrap();
                // inmsg
                let in_msg_ex = InMsg::external_msg(
                    inmsg.serialize().unwrap(),
                    transaction.serialize().unwrap(),
                );

                // outmsgs
                let mut value = CurrencyCollection::default();
                value.grams = 10202u64.into();
                let mut imh = InternalMessageHeader::with_addresses(
                    MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    MsgAddressInt::with_standart(None, 0, AccountId::from_raw(vec![255; 32], 256))
                        .unwrap(),
                    value,
                );

                imh.ihr_fee = 10u64.into();
                imh.fwd_fee = 5u64.into();
                let outmsg1 = Message::with_int_header(imh);

                let ext_out_header = ExtOutMessageHeader::with_addresses(
                    MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80]))
                        .unwrap(),
                );

                let mut outmsg2 = Message::with_ext_out_header(ext_out_header);
                outmsg2.set_body(SliceData::new(vec![0x02; 120]));

                let tr_cell = transaction.serialize().unwrap();

                let env = MsgEnvelope::with_message_and_fee(&outmsg1, 9u64.into()).unwrap();
                let out_msg1 = OutMsg::new_msg(env.serialize().unwrap(), tr_cell.clone());

                let out_msg2 = OutMsg::external_msg(outmsg2.serialize().unwrap(), tr_cell);

                let in_msg = Arc::new(if rng.gen() { in_msg_int } else { in_msg_ex });
                // builder can stop earlier than writing threads it is not a problem here
                if !builder_clone.add_transaction(in_msg, vec![out_msg1, out_msg2]) {
                    break;
                }

                thread::sleep(Duration::from_millis(1)); // emulate timeout working TVM
            }
        });
    }

    thread::sleep(Duration::from_millis(10));

    let ss = ShardStateUnsplit::default();

    block_builder.finalize_block(&ss, &ss).unwrap()
}
