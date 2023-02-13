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

use crate::data::{
    BlocksStorage, DocumentsDb, FileBasedStorage, FinalityStorage, SerializedItem, ShardStateInfo,
    ShardStateStorage, TransactionsStorage,
};
use crate::error::NodeError;
use crate::NodeResult;
use std::collections::{BTreeMap, HashSet};
use std::{
    collections::HashMap,
    fs::{create_dir_all, File},
    io::{ErrorKind, Read, Seek, Write},
    path::PathBuf,
    sync::Arc,
};
use ton_block::*;
use ton_types::*;

#[cfg(test)]
#[path = "../../../../tonos-se-tests/unit/test_block_finality.rs"]
mod tests;

lazy_static::lazy_static!(
    static ref ACCOUNT_NONE_HASH: UInt256 = Account::default().serialize().unwrap().repr_hash();
    pub static ref MINTER_ADDRESS: MsgAddressInt =
        MsgAddressInt::AddrStd(MsgAddrStd::with_address(None, 0, [0; 32].into()));
);

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
    _tr_storage: Arc<T>,
    fn_storage: Arc<F>,
    db: Option<Arc<dyn DocumentsDb>>,

    current_block: Box<ShardBlock>,
    blocks_by_hash: HashMap<UInt256, Box<FinalityBlock>>,
    // need to remove
    blocks_by_no: HashMap<u64, Box<FinalityBlock>>,
    // need to remove
    pub(crate) last_finalized_block: Box<ShardBlock>,
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
        global_id: i32,
        shard_ident: ShardIdent,
        root_path: PathBuf,
        shard_state_storage: Arc<S>,
        blocks_storage: Arc<B>,
        _tr_storage: Arc<T>,
        fn_storage: Arc<F>,
        db: Option<Arc<dyn DocumentsDb>>,
        _public_keys: Vec<ed25519_dalek::PublicKey>,
    ) -> Self {
        let root_path = FileBasedStorage::create_workchains_dir(&root_path)
            .expect("cannot create shards directory");

        OrdinaryBlockFinality {
            shard_ident: shard_ident.clone(),
            root_path,
            shard_state_storage,
            blocks_storage,
            _tr_storage,
            fn_storage,
            db,
            current_block: Box::new(ShardBlock::new(global_id, shard_ident.clone())),
            blocks_by_hash: HashMap::new(),
            blocks_by_no: HashMap::new(),
            last_finalized_block: Box::new(ShardBlock::new(global_id, shard_ident.clone())),
        }
    }

    pub(crate) fn get_last_info(&self) -> NodeResult<(Arc<ShardStateUnsplit>, BlkPrevInfo)> {
        Ok((self.get_last_shard_state(), self.get_last_block_info()?))
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

            if let Err(err) = self.reflect_block_in_db(&sb) {
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
            Ok(Box::new(ShardBlock::new(
                self.current_block.block.global_id,
                self.shard_ident.clone(),
            )))
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

    fn prepare_messages_from_transaction(
        transaction: &Transaction,
        block_id: UInt256,
        tr_chain_order: &str,
        block_root_for_proof: Option<&Cell>,
        messages: &mut HashMap<UInt256, serde_json::value::Map<String, serde_json::Value>>,
    ) -> Result<()> {
        if let Some(message_cell) = transaction.in_msg_cell() {
            let message = Message::construct_from_cell(message_cell.clone())?;
            let message_id = message_cell.repr_hash();
            let mut doc =
                if message.is_inbound_external() || message.src_ref() == Some(&MINTER_ADDRESS) {
                    Self::prepare_message_json(
                        message_cell,
                        message,
                        block_root_for_proof,
                        block_id.clone(),
                        Some(transaction.now()),
                    )?
                } else {
                    messages.remove(&message_id).unwrap_or_else(|| {
                        let mut doc = serde_json::value::Map::with_capacity(2);
                        doc.insert("id".into(), message_id.as_hex_string().into());
                        doc
                    })
                };

            doc.insert(
                "dst_chain_order".into(),
                format!("{}{}", tr_chain_order, ton_block_json::u64_to_string(0)).into(),
            );

            messages.insert(message_id, doc);
        };

        let mut index: u64 = 1;
        transaction.out_msgs.iterate_slices(|slice| {
            let message_cell = slice.reference(0)?;
            let message_id = message_cell.repr_hash();
            let message = Message::construct_from_cell(message_cell.clone())?;
            let mut doc = Self::prepare_message_json(
                message_cell,
                message,
                block_root_for_proof,
                block_id.clone(),
                None, // transaction_now affects ExtIn messages only
            )?;

            // messages are ordered by created_lt
            doc.insert(
                "src_chain_order".into(),
                format!("{}{}", tr_chain_order, ton_block_json::u64_to_string(index)).into(),
            );

            index += 1;
            messages.insert(message_id, doc);
            Ok(true)
        })?;

        Ok(())
    }

    fn prepare_message_json(
        message_cell: Cell,
        message: Message,
        block_root_for_proof: Option<&Cell>,
        block_id: UInt256,
        transaction_now: Option<u32>,
    ) -> Result<serde_json::value::Map<String, serde_json::Value>> {
        let boc = serialize_toc(&message_cell)?;
        let proof = block_root_for_proof
            .map(|cell| serialize_toc(&message.prepare_proof(true, cell)?))
            .transpose()?;

        let set = ton_block_json::MessageSerializationSet {
            message,
            id: message_cell.repr_hash(),
            block_id: Some(block_id.clone()),
            transaction_id: None, // it would be ambiguous for internal or replayed messages
            status: MessageProcessingStatus::Finalized,
            boc,
            proof,
            transaction_now, // affects ExtIn messages only
        };
        let doc = ton_block_json::db_serialize_message("id", &set)?;
        Ok(doc)
    }

    fn doc_to_item(doc: serde_json::value::Map<String, serde_json::Value>) -> SerializedItem {
        SerializedItem {
            id: doc["id"].as_str().unwrap().to_owned(),
            data: serde_json::json!(doc),
        }
    }

    fn prepare_transaction_json(
        tr_cell: Cell,
        transaction: Transaction,
        block_root: &Cell,
        block_id: UInt256,
        workchain_id: i32,
        add_proof: bool,
    ) -> Result<serde_json::value::Map<String, serde_json::Value>> {
        let boc = serialize_toc(&tr_cell).unwrap();
        let proof = if add_proof {
            Some(serialize_toc(&transaction.prepare_proof(&block_root)?)?)
        } else {
            None
        };
        let set = ton_block_json::TransactionSerializationSet {
            transaction,
            id: tr_cell.repr_hash(),
            status: TransactionProcessingStatus::Finalized,
            block_id: Some(block_id.clone()),
            workchain_id,
            boc,
            proof,
            ..Default::default()
        };
        let doc = ton_block_json::db_serialize_transaction("id", &set)?;
        Ok(doc)
    }

    fn prepare_account_record(
        account: Account,
        prev_account_state: Option<Account>,
        last_trans_chain_order: Option<String>,
    ) -> Result<SerializedItem> {
        let mut boc1 = None;
        if account.init_code_hash().is_some() {
            // new format
            let mut builder = BuilderData::new();
            account.write_original_format(&mut builder)?;
            boc1 = Some(serialize_toc(&builder.into_cell()?)?);
        }
        let boc = serialize_toc(&account.serialize()?.into())?;

        let set = ton_block_json::AccountSerializationSet {
            account,
            prev_account_state,
            proof: None,
            boc,
            boc1,
            ..Default::default()
        };

        let mut doc = ton_block_json::db_serialize_account("id", &set)?;
        if let Some(last_trans_chain_order) = last_trans_chain_order {
            doc.insert(
                "last_trans_chain_order".to_owned(),
                last_trans_chain_order.into(),
            );
        }
        Ok(Self::doc_to_item(doc))
    }

    fn prepare_deleted_account_record(
        account_id: AccountId,
        workchain_id: i32,
        prev_account_state: Option<Account>,
        last_trans_chain_order: Option<String>,
    ) -> Result<SerializedItem> {
        let set = ton_block_json::DeletedAccountSerializationSet {
            account_id,
            workchain_id,
            prev_account_state,
            ..Default::default()
        };

        let mut doc = ton_block_json::db_serialize_deleted_account("id", &set)?;
        if let Some(last_trans_chain_order) = last_trans_chain_order {
            doc.insert(
                "last_trans_chain_order".to_owned(),
                last_trans_chain_order.into(),
            );
        }
        Ok(Self::doc_to_item(doc))
    }

    fn prepare_block_record(
        block: &Block,
        block_root: &Cell,
        boc: &[u8],
        file_hash: &UInt256,
        block_order: String,
    ) -> Result<SerializedItem> {
        let set = ton_block_json::BlockSerializationSetFH {
            block,
            id: &block_root.repr_hash(),
            status: BlockProcessingStatus::Finalized,
            boc,
            file_hash: Some(file_hash),
        };
        let mut doc = ton_block_json::db_serialize_block("id", set)?;
        doc.insert("chain_order".to_owned(), block_order.into());
        Ok(Self::doc_to_item(doc))
    }

    pub fn reflect_block_in_db(&self, shard_block: &ShardBlock) -> Result<()> {
        let db = match self.db.clone() {
            Some(db) => db,
            None => return Ok(()),
        };

        let add_proof = false;
        let block_id = &shard_block.root_hash;
        log::trace!("Processor block_stuff.id {}", block_id.to_hex_string());
        let file_hash = shard_block.file_hash.clone();
        let block = &shard_block.block;
        let block_root = shard_block.block.serialize()?;
        let block_extra = block.read_extra()?;
        let block_boc = &shard_block.serialized_block;
        let info = block.read_info()?;
        let block_order = ton_block_json::block_order(&block, info.seq_no())?;
        let workchain_id = info.shard().workchain_id();
        let shard_accounts = shard_block.shard_state.read_accounts()?;

        let now_all = std::time::Instant::now();

        // Prepare sorted ton_block transactions and addresses of changed accounts
        let mut changed_acc = HashSet::new();
        let mut deleted_acc = HashSet::new();
        let mut acc_last_trans_chain_order = HashMap::new();
        let now = std::time::Instant::now();
        let mut tr_count = 0;
        let mut transactions = BTreeMap::new();
        block_extra
            .read_account_blocks()?
            .iterate_objects(|account_block: AccountBlock| {
                // extract ids of changed accounts
                let state_upd = account_block.read_state_update()?;
                let mut check_account_existed = false;
                if state_upd.new_hash == *ACCOUNT_NONE_HASH {
                    deleted_acc.insert(account_block.account_id().clone());
                    if state_upd.old_hash == *ACCOUNT_NONE_HASH {
                        check_account_existed = true;
                    }
                } else {
                    changed_acc.insert(account_block.account_id().clone());
                }

                let mut account_existed = false;
                account_block
                    .transactions()
                    .iterate_slices(|_, transaction_slice| {
                        // extract transactions
                        let cell = transaction_slice.reference(0)?;
                        let transaction =
                            Transaction::construct_from(&mut SliceData::load_cell(cell.clone())?)?;
                        let ordering_key =
                            (transaction.logical_time(), transaction.account_id().clone());
                        if transaction.orig_status != AccountStatus::AccStateNonexist
                            || transaction.end_status != AccountStatus::AccStateNonexist
                        {
                            account_existed = true;
                        }
                        transactions.insert(ordering_key, (cell, transaction));
                        tr_count += 1;

                        Ok(true)
                    })?;

                if check_account_existed && !account_existed {
                    deleted_acc.remove(account_block.account_id());
                }

                Ok(true)
            })?;
        log::trace!(
            "TIME: preliminary prepare {} transactions {}ms;   {}",
            tr_count,
            now.elapsed().as_millis(),
            block_id
        );

        // Iterate ton_block transactions to:
        // - prepare messages and transactions for external db
        // - prepare last_trans_chain_order for accounts
        let now = std::time::Instant::now();
        let mut index = 0;
        let mut messages = Default::default();
        for (_, (cell, transaction)) in transactions.into_iter() {
            let tr_chain_order = format!(
                "{}{}",
                block_order,
                ton_block_json::u64_to_string(index as u64)
            );

            Self::prepare_messages_from_transaction(
                &transaction,
                block_root.repr_hash(),
                &tr_chain_order,
                add_proof.then(|| &block_root),
                &mut messages,
            )?;

            let account_id = transaction.account_id().clone();
            acc_last_trans_chain_order.insert(account_id, tr_chain_order.clone());

            let mut doc = Self::prepare_transaction_json(
                cell,
                transaction,
                &block_root,
                block_root.repr_hash(),
                workchain_id,
                add_proof,
            )?;
            doc.insert("chain_order".into(), tr_chain_order.into());
            db.put_transaction(Self::doc_to_item(doc))?;

            index += 1;
        }
        let msg_count = messages.len(); // is 0 if not process_message
        for (_, message) in messages {
            db.put_message(Self::doc_to_item(message))?;
        }
        log::trace!(
            "TIME: prepare {} transactions and {} messages {}ms;   {}",
            tr_count,
            msg_count,
            now.elapsed().as_millis(),
            block_id,
        );

        // Prepare accounts (changed and deleted)
        let now = std::time::Instant::now();
        for account_id in changed_acc.iter() {
            let acc = shard_accounts.account(account_id)?.ok_or_else(|| {
                NodeError::InvalidData(
                    "Block and shard state mismatch: \
                        state doesn't contain changed account"
                        .to_string(),
                )
            })?;
            let acc = acc.read_account()?;

            db.put_account(Self::prepare_account_record(
                acc,
                None,
                acc_last_trans_chain_order.remove(account_id),
            )?)?;
        }

        for account_id in deleted_acc {
            let last_trans_chain_order = acc_last_trans_chain_order.remove(&account_id);
            db.put_account(Self::prepare_deleted_account_record(
                account_id,
                workchain_id,
                None,
                last_trans_chain_order,
            )?)?;
        }
        log::trace!(
            "TIME: accounts {} {}ms;   {}",
            changed_acc.len(),
            now.elapsed().as_millis(),
            block_id
        );

        // Block
        let now = std::time::Instant::now();
        db.put_block(Self::prepare_block_record(
            block,
            &block_root,
            block_boc,
            &file_hash,
            block_order.clone(),
        )?)?;
        log::trace!(
            "TIME: block {}ms;   {}",
            now.elapsed().as_millis(),
            block_id
        );
        log::trace!(
            "TIME: prepare & build jsons {}ms;   {}",
            now_all.elapsed().as_millis(),
            block_id
        );
        Ok(())
    }

    /// Save block until finality comes
    pub(crate) fn put_block_with_info(
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

    #[cfg(test)]
    pub fn find_block_by_hash(&self, hash: &UInt256) -> u64 {
        if self.blocks_by_hash.contains_key(hash) {
            self.blocks_by_hash.get(hash).unwrap().seq_no()
        } else {
            0xFFFFFFFF // if not found
        }
    }

    /// Rollback shard state to one of block candidates
    #[cfg(test)]
    pub fn rollback_to(&mut self, hash: &UInt256) -> NodeResult<()> {
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

    #[cfg(test)]
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
    pub(crate) block: Block,
    shard_state: Arc<ShardStateUnsplit>,
}

impl ShardBlock {
    fn new(global_id: i32, shard: ShardIdent) -> Self {
        let mut shard_state = ShardStateUnsplit::default();
        shard_state.set_global_id(global_id);
        shard_state.set_shard(shard);
        let mut block = Block::default();
        block.global_id = global_id;
        Self {
            seq_no: 0,
            serialized_block: Vec::new(),
            root_hash: UInt256::ZERO,
            file_hash: UInt256::ZERO,
            block,
            shard_state: Arc::new(shard_state),
        }
    }

    /// get current block sequence number
    pub fn get_seq_no(&self) -> u64 {
        self.seq_no
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
        let mut sb = ShardBlock::new(0, ShardIdent::default());
        sb.seq_no = rdr.read_le_u64()?;
        let sb_len = rdr.read_le_u32()?;
        let mut sb_buf = vec![0; sb_len as usize];
        rdr.read(&mut sb_buf)?;
        sb.serialized_block = sb_buf;

        let hash = rdr.read_u256()?;
        sb.root_hash = UInt256::from(hash);

        let hash = rdr.read_u256()?;
        sb.file_hash = UInt256::from(hash);

        let mut shard_slice = SliceData::load_cell(deserialize_tree_of_cells(rdr)?)?;
        sb.shard_state.read_from(&mut shard_slice)?;

        let cell = deserialize_tree_of_cells(rdr)?;
        sb.block = Block::construct_from_cell(cell)?;
        Ok(sb)
    }
}

// runs 10 thread to generate 5000 accounts with 1 input and two output messages per every block
// finalizes block and return
#[cfg(test)]
pub fn generate_block_with_seq_no(
    shard_ident: ShardIdent,
    seq_no: u32,
    prev_info: BlkPrevInfo,
) -> Block {
    let mut block_builder = crate::block::builder::BlockBuilder::with_shard_ident(
        shard_ident,
        seq_no,
        prev_info,
        UnixTime32::now().as_u32(),
    );

    //println!("Thread write start.");
    for _ in 0..5000 {
        let acc = AccountId::from_raw(
            (0..32).map(|_| rand::random::<u8>()).collect::<Vec<u8>>(),
            256,
        );
        let mut transaction =
            Transaction::with_address_and_status(acc.clone(), AccountStatus::AccStateActive);
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
        let mut inmsg1 = Message::with_int_header(imh);
        inmsg1.set_body(SliceData::new(vec![0x21; 120]));

        let ext_in_header = ExternalInboundMessageHeader {
            src: MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80]))
                .unwrap(),
            dst: MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            import_fee: 10u64.into(),
        };

        let mut inmsg = Message::with_ext_in_header(ext_in_header);
        inmsg.set_body(SliceData::new(vec![0x01; 120]));

        transaction.write_in_msg(Some(&inmsg1)).unwrap();

        // outmsgs
        let mut value = CurrencyCollection::default();
        value.grams = 10202u64.into();
        let mut imh = InternalMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            MsgAddressInt::with_standart(None, 0, AccountId::from_raw(vec![255; 32], 256)).unwrap(),
            value,
        );

        imh.ihr_fee = 10u64.into();
        imh.fwd_fee = 5u64.into();
        let outmsg1 = Message::with_int_header(imh);

        let ext_out_header = ExtOutMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80])).unwrap(),
        );

        let mut outmsg2 = Message::with_ext_out_header(ext_out_header);
        outmsg2.set_body(SliceData::new(vec![0x02; 120]));

        transaction.add_out_message(&outmsg1).unwrap();
        transaction.add_out_message(&outmsg2).unwrap();

        let tr_cell = transaction.serialize().unwrap();
        block_builder
            .add_raw_transaction(transaction, tr_cell)
            .unwrap();
    }
    let (block, _) = block_builder.finalize_block().unwrap();
    block
}
