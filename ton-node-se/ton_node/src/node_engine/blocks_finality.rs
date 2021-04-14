use super::*;
use crate::error::{NodeError, NodeErrorKind};
use poa::engines::authority_round::RollingFinality;
use rand::Rng;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{create_dir_all, File};
use std::io::{ErrorKind, Read, Seek, Write};
use std::path::PathBuf;
use ton_block::{InMsg, OutMsg, AccountStatus, ExtOutMessageHeader, MsgEnvelope};
use ton_block::{HashmapAugType, BlkPrevInfo, Deserializable, ShardIdent};
use ton_types::{UInt256, deserialize_tree_of_cells, BagOfCells, BuilderData};

#[cfg(test)]
#[path = "../../../tonos-se-tests/unit/test_block_finality.rs"]
mod tests;

/// Structure for Block finality layer
/// This realize next functionality:
/// Store block received from POA
/// Finalize block witch POA finality - store to permanent storage
/// Store (in memory) current shard-state for every block-candidate
/// Get current shard-state
/// Get last block by number or hash
/// Rollback block (and shardstate) if block is rolled
#[allow(dead_code)]
pub struct OrdinaryBlockFinality<S, B, T, F> where
    S: ShardStateStorage,
    B: BlocksStorage,
    T: TransactionsStorage,
    F: FinalityStorage
{
    shard_ident: ShardIdent,
    root_path: PathBuf,
    shard_state_storage: Arc<S>,
    blocks_storage: Arc<B>,
    tr_storage: Arc<T>,
    fn_storage: Arc<F>,
    db: Option<Arc<Box<dyn DocumentsDb>>>,
    
    pub finality: RollingFinality,
    current_block: Box<ShardBlock>,
    blocks_by_hash: HashMap<UInt256, Box<FinalityBlock>>,
    blocks_by_no: HashMap<u64, Box<FinalityBlock>>,
    last_finalized_block: Box<ShardBlock>,
}

fn key_by_seqno(seq_no: u32, vert_seq_no: u32) -> u64 {
    ((vert_seq_no as u64) << 32) | (seq_no as u64)
}

impl <S, B, T, F> OrdinaryBlockFinality<S, B, T, F> where
    S: ShardStateStorage,
    B: BlocksStorage,
    T: TransactionsStorage,
    F: FinalityStorage
{
    /// Create new instance BlockFinality
    /// wtih all kind of storages
    pub fn with_params(
        shard_ident: ShardIdent,
        root_path: PathBuf,
        shard_state_storage: Arc<S>, 
        blocks_storage: Arc<B>, 
        tr_storage: Arc<T>,
        fn_storage: Arc<F>,
        db: Option<Arc<Box<dyn DocumentsDb>>>,
        public_keys: Vec<ed25519_dalek::PublicKey>
    ) -> Self {
        let root_path = FileBasedStorage::create_workchains_dir(&root_path)
                        .expect("cannot create shardes directory");
       
        OrdinaryBlockFinality {
            shard_ident,
            root_path,
            shard_state_storage,
            blocks_storage,
            tr_storage,
            fn_storage,
            db,
            finality: RollingFinality::blank(public_keys),
            current_block: Box::new(ShardBlock::default()),
            blocks_by_hash: HashMap::new(),
            blocks_by_no: HashMap::new(),
            last_finalized_block: Box::new(ShardBlock::default()),
        }
    } 

    fn finality_blocks(&mut self, hashes: Vec<UInt256>, is_sync: bool) -> NodeResult<()> {
debug!("FINBLK {:?}", hashes);
        for hash in hashes.iter() {
            if self.blocks_by_hash.contains_key(&hash) {
                let fin_sb = self.blocks_by_hash.remove(&hash).unwrap();
                
                let sb = self.stored_to_loaded(fin_sb)?;

                // create shardstateinfo
                let info = sb.block.block().read_info()?;
                let shard_info = ShardStateInfo {
                    seq_no: key_by_seqno(info.seq_no(), info.vert_seq_no()),
                    lt: info.end_lt(),
                    hash: sb.block_hash.clone(),
                };
                let mut shard = vec![];
                BagOfCells::with_root(&sb.shard_state.write_to_new_cell()?.into())
                    .write_to(&mut shard, false)?;
                // save shard state
                self.shard_state_storage.save_serialized_shardstate_ex(
                    &ShardStateUnsplit::default(), Some(shard), &sb.block.block().read_state_update()?.new_hash,
                    shard_info
                )?;
                // save finalized block
                self.blocks_storage.save_raw_block(&sb.block, Some(&sb.serialized_block))?;
                // remove shard-block from number hashmap
                self.blocks_by_no.remove(&sb.seq_no).expect("block by number remove error");
                // remove old_last_finality block from finality storage
                self.remove_form_finality_storage(&self.last_finalized_block.block_hash)?;

                let res = self.reflect_block_in_db(
                    sb.block.block().clone(),
                    sb.shard_state.clone(),
                    is_sync,
                    BlockProcessingStatus::Finalized,
                    MessageProcessingStatus::Finalized,
                    TransactionProcessingStatus::Finalized
                ); 

                if res.is_err() {
                   warn!(target: "node", "reflect_block_in_db(Finalized) error: {:?}", res.unwrap_err());
                }

                self.last_finalized_block = sb;
                info!(target: "node", "FINALITY:save block seq_no: {:?}, serialized len = {}",
                    self.last_finalized_block.block.block().read_info()?.seq_no(),
                    self.last_finalized_block.serialized_block.len()
                );
            } else {
                if hash != &UInt256::from([0;32]) && hash != &self.last_finalized_block.block_hash {
                    warn!(target: "node", "Can`t finality unknown hash!!!");
                    return node_err!(NodeErrorKind::FinalityError)
                }
            }
        }
        Ok(())
    }

    fn stored_to_loaded(&self, fin_sb: Box<FinalityBlock> ) -> NodeResult<Box<ShardBlock>> {
        Ok(match *fin_sb {
            FinalityBlock::Stored(sb_hash) => {
                // load sb
                let (shard_path, _blocks_path, _tr_dir) = 
                    FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
                let mut block_sb_path = shard_path.clone();
                block_sb_path.push("block_finality");
                if !block_sb_path.as_path().exists() {
                    create_dir_all(block_sb_path.as_path())?;
                }     

                block_sb_path.push(sb_hash.block_hash.to_hex_string());
                
                self.read_one_sb_from_file(block_sb_path)?
            },
            FinalityBlock::Loaded(l_sb) => {
                l_sb
            }
        })
    }

    pub fn save_rolling_finality(&self) -> NodeResult<()> {
        // save to disk
        let (mut finality_file_name, _blocks_path, _tr_dir) = 
            FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
        finality_file_name.push("rolling_finality.info");
        self.finality.save(finality_file_name.to_str().expect("Can`t get rolling finality file name."))?;
        Ok(())        
    }

    fn remove_form_finality_storage(&self, hash: &UInt256) -> NodeResult<()>{
        let (shard_path, _blocks_path, _tr_dir) = 
            FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
        let mut block_finality_path = shard_path.clone();
        block_finality_path.push("block_finality");
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

    fn save_one(sb: Box<ShardBlock>, file_name: PathBuf) -> NodeResult<()> {
        let mut file_info = File::create(file_name)?;
        let mut data = sb.serialize()?;
        file_info.write_all(&mut data[..])?;
        file_info.flush()?; 
        Ok(())
    }

    pub fn save_finality(&self) -> NodeResult<()> {
        let (shard_path, _blocks_path, _tr_dir) = 
            FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
        let mut block_finality_path = shard_path.clone();
        block_finality_path.push("block_finality");
        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }       
        let mut name = block_finality_path.clone();
        name.push(self.current_block.block_hash.to_hex_string());
        if !name.as_path().exists() {
            Self::save_one(self.current_block.clone(), name)?;
        }
        let mut name = block_finality_path.clone();
        name.push(self.last_finalized_block.block_hash.to_hex_string());
        if !name.as_path().exists() {
            Self::save_one(self.last_finalized_block.clone(), name)?;
        }
        self.save_index()?;
        Ok(())
    }

    fn save_index(&self) -> NodeResult<()> {
        let (mut shard_path, _blocks_path, _tr_dir) = 
            FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
        shard_path.push("blocks_finality.info");
        let mut file_info = File::create(shard_path)?;
        self.serialize(&mut file_info)?;
        file_info.flush()?;
        self.save_rolling_finality()?;
        Ok(())
    }

    ///
    /// Load from disk
    /// 
    pub fn load(&mut self) -> NodeResult<()> {
        let (mut shard_path, _blocks_path, _tr_dir) = 
            FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
        let mut finality_file_name = shard_path.clone();
        shard_path.push("blocks_finality.info");
        info!(target: "node", "load: {}", shard_path.to_str().unwrap());
        let mut file_info = File::open(shard_path)?;
        self.deserialize(&mut file_info)?;
        finality_file_name.push("rolling_finality.info");
        info!(target: "node", "load: {}", finality_file_name.to_str().unwrap());
        self.finality.load(finality_file_name.to_str().expect("Can`t get rolling finality file name."))?;
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
        writer.write_all(self.current_block.block_hash.as_slice())?;
        writer.write_all(&self.current_block.seq_no.to_le_bytes())?;
        // save last finality block
        writer.write_all(self.last_finalized_block.block_hash.as_slice())?;
        writer.write_all(&self.last_finalized_block.seq_no.to_le_bytes())?;
        for (key, sb) in self.blocks_by_hash.iter() {
            writer.write_all(key.as_slice())?;
            writer.write_all(&sb.seq_no().to_le_bytes())?;
        }
        Ok(())
    }

    fn read_one_sb_hash<R>(&self, rdr: &mut R) -> NodeResult<(u64, UInt256)> where R: Read + Seek {
        // first read current block
        let hash = rdr.read_u256()?;
        let seq_no = rdr.read_le_u64()?;
        Ok((seq_no, UInt256::from(hash)))
    }

    fn read_one_sb<R>(&self, rdr: &mut R) -> NodeResult<Box<ShardBlock>> where R: Read + Seek {
        let (shard_path, _blocks_path, _tr_dir) = 
            FileBasedStorage::create_default_shard_catalog(self.root_path.clone(), &self.shard_ident)?;
        let mut block_finality_path = shard_path.clone();
        block_finality_path.push("block_finality");
        if !block_finality_path.as_path().exists() {
            create_dir_all(block_finality_path.as_path())?;
        }     
        // first read current block
        let hash = UInt256::from(rdr.read_u256()?);
        let seq_no = rdr.read_le_u64()?;
        info!(target: "node", "read_one_sb:seq_no: {}", seq_no);
        if seq_no == 0 {
            Ok(Box::new(ShardBlock::default()))
        } else {
            let mut current_block_name = block_finality_path.clone();
            current_block_name.push(hash.to_hex_string());

            self.read_one_sb_from_file(current_block_name)
        }
    }

    fn read_one_sb_from_file(&self, file_name: PathBuf) -> NodeResult<Box<ShardBlock>> {
        info!(target: "node", "load {}", file_name.to_str().unwrap());
        let mut file_info = File::open(file_name.clone())?;
        let mut data = Vec::new();
        file_info.read_to_end(&mut data)?;
        info!(target: "node", "load {} ok.", file_name.to_str().unwrap());
        Ok(Box::new(ShardBlock::deserialize(&mut std::io::Cursor::new(data))?))
    }

    ///
    /// Read saved data BlockFinality from file
    /// 
    pub fn deserialize<R>(&mut self, rdr: &mut R) -> NodeResult<()> where R: Read + Seek {

        info!(target: "node", "load current block");
        self.current_block = self.read_one_sb(rdr)?;
        info!(target: "node", "load last finalized block");
        self.last_finalized_block = self.read_one_sb(rdr)?;
        loop {
            info!(target: "node", "load non finalized block");
            let res = self.read_one_sb_hash(rdr);
            if res.is_ok() {
                let (seq_no, hash) = res.unwrap();
                let sb_hash = Box::new(FinalityBlock::Stored(
                    Box::new(ShardBlockHash::with_hash(seq_no.clone(), hash.clone()))
                ));

                self.blocks_by_hash.insert(hash.clone(), sb_hash.clone());
                self.blocks_by_no.insert(seq_no.clone(), sb_hash.clone());
            } else {
                match res.unwrap_err() {
                    NodeError(NodeErrorKind::Io(err), _) => {
                        if err.kind() == ErrorKind::UnexpectedEof {
                            break;
                        }
                    },
                    err => {
                        bail!(err);
                    }
                }
            }     
        }
        Ok(())
    }

    /// save objects into kafka with "finalized" state,
    fn reflect_block_in_db(
        &self,
        block: Block,
        shard_state: Arc<ShardStateUnsplit>,
        is_sync: bool, 
        block_status: BlockProcessingStatus,
        msg_status: MessageProcessingStatus,
        tr_status: TransactionProcessingStatus) -> NodeResult<()> {
        
        if !is_sync {
            if let Some(db) = self.db.clone() {

                // let block_root = block.write_to_new_cell()?.into();
                // let state_root = shard_state.write_to_new_cell()?.into();
                // let block_info_root = block.read_info()?.write_to_new_cell()?;
                // let block_info_cells = BagOfCells::with_root(&block_info_root.into())
                //     .withdraw_cells();

                let block_id = block.hash()?;
                let extra = block.read_extra()?;
                let workchain_id = block.read_info()?.shard().workchain_id();

                extra.read_in_msg_descr()?.iterate_objects(|in_msg| {
                    let msg = in_msg.read_message()?;
debug!(target: "node", "PUT-IN-MESSAGE-BLOCK {}", msg.hash()?.to_hex_string());
                    // msg.prepare_proof_for_json(&block_info_cells, &block_root)?;
                    // msg.prepare_boc_for_json()?;
                    let transaction_id = in_msg.transaction_cell().map(|cell| cell.repr_hash());
                    let transaction_now = in_msg.read_transaction()?.map(|transaction| transaction.now());
                    let res = db.put_message(msg, msg_status, transaction_id, transaction_now, Some(block_id.clone()));
                    if res.is_err() {
                        warn!(target: "node", "reflect message to DB(1). error: {}", res.unwrap_err());
                    }
                    Ok(true)
                })?;

                debug!(target: "node", "in_msg_descr.iterate - success");


                extra.read_out_msg_descr()?.iterate_objects(|out_msg| {
                    let msg = out_msg.read_message()?.unwrap();
debug!(target: "node", "PUT-OUT-MESSAGE-BLOCK {:?}", msg);
                    // msg1.prepare_proof_for_json(&block_info_cells, &block_root)?;
                    // msg1.prepare_boc_for_json()?;
                    let transaction_id = out_msg.transaction_cell().map(|cell| cell.repr_hash());
                    let res = db.put_message(msg, msg_status, transaction_id, None, Some(block_id.clone()));
                    if res.is_err() {
                        warn!(target: "node", "reflect message to DB(2). error: {}", res.unwrap_err());
                    }
                    Ok(true)
                })?;

                debug!(target: "node", "out_msg_descr.iterate - success");

                let mut changed_acc = HashSet::new();

                extra.read_account_blocks()?.iterate_with_keys(|account_id, account_block| {
                    let mut orig_status = None;
                    let mut end_status = None;
                    changed_acc.insert(account_id);
                    account_block.transaction_iterate(|transaction| {
                        // transaction.prepare_proof_for_json(&block_info_cells, &block_root)?;
                        // transaction.prepare_boc_for_json()?;
debug!(target: "node", "PUT-TRANSACTION-BLOCK {}", transaction.hash()?.to_hex_string());
                        if orig_status.is_none() {
                            orig_status = Some(transaction.orig_status.clone());
                        }
                        end_status = Some(transaction.end_status.clone());
                        if let Err(err) = db.put_transaction(transaction, tr_status, Some(block_id.clone()), workchain_id) {
                            warn!(target: "node", "reflect transaction to DB. error: {}", err);
                        }
                        Ok(true)
                    })?;
                    if end_status == Some(AccountStatus::AccStateNonexist) &&
                        end_status != orig_status
                    {
                        let account_id = account_block.account_id().clone();
                        if let Err(err) = db.put_deleted_account(workchain_id, account_id) {
                            warn!(target: "node", "reflect deleted account to DB. error: {}", err);
                        }
                    }
                    Ok(true)
                })?;

                //if block_status == BlockProcessingStatus::Finalized {
                shard_state.read_accounts()?.iterate_with_keys(|id, acc| {
                    if changed_acc.contains(&id) {
                        // acc.account.prepare_proof_for_json(&state_root)?;
                        // acc.account.prepare_boc_for_json()?;
                        match acc.read_account()? {
                            Account::AccountNone => {
                                error!(target: "node", "something gone wrong with account {:x}", id);
                            }
                            acc => if let Err(err) = db.put_account(acc) {
                                warn!(target: "node", "reflect account to DB. error: {}", err);
                            }         
                        }
                    }
                    Ok(true)
                })?;
                //}

                debug!(target: "node", "accounts.iterate - success");

                db.put_block(block, block_status.clone())?;
            }
        }
        Ok(())
    }
}

impl<S, B, T, F> BlockFinality for OrdinaryBlockFinality<S, B, T, F> where
    S: ShardStateStorage,
    B: BlocksStorage,
    T: TransactionsStorage,
    F: FinalityStorage,
{
    /// finalize block through empty step
    fn finalize_without_new_block(&mut self, finality_hash: Vec<UInt256>) -> NodeResult<()> {
debug!(target: "node", "NO-BLOCK {:?}", finality_hash);        
        self.finality_blocks(finality_hash, false)?;
        self.save_finality()?;
        Ok(())
    }

    /// Save block until finality comes 
    fn put_block_with_info(&mut self,
        sblock: SignedBlock,
        sblock_data:Option<Vec<u8>>,
        block_hash: Option<UInt256>,
        shard_state: Arc<ShardStateUnsplit>,
        finality_hash: Vec<UInt256>,
        is_sync: bool,
    ) -> NodeResult<()> 
    {
        info!(target: "node", "FINALITY: add block. hash: {:?}", block_hash);
        info!(target: "node", "FINALITY:    block seq_no: {:?}", sblock.block().read_info()?.seq_no());

debug!(target: "node", "PUT-BLOCK {:?}", finality_hash);        
        let res = self.reflect_block_in_db(
            sblock.block().clone(),
            shard_state.clone(),
            is_sync,
            BlockProcessingStatus::Proposed,
            MessageProcessingStatus::Proposed,
            TransactionProcessingStatus::Proposed
        );

        if res.is_err() {
            warn!(target: "node", "reflect_block_in_db(Proposed) error: {:?}", res.unwrap_err());
        }

        let sb = Box::new(ShardBlock::with_block_and_state(
            sblock,
            sblock_data,
            block_hash,
            shard_state,
        ));
debug!(target: "node", "PUT-BLOCK-HASH {:?}", sb.block_hash);        
        
        self.current_block = sb.clone();
        
        // insert block to map
        let mut loaded = false;
        for hash in finality_hash.iter() {
            if hash == &sb.block_hash {
                loaded = true;
                break;
            }
        }
        let sb_hash = if loaded {
            Box::new(FinalityBlock::Loaded(sb.clone()))
        } else {
            Box::new(
                FinalityBlock::Stored(
                    Box::new( 
                        ShardBlockHash::with_hash(sb.seq_no.clone(), sb.block_hash.clone())
                    )
                )
            )
        };
        self.blocks_by_hash.insert(sb.block_hash.clone(), sb_hash.clone());
        self.blocks_by_no.insert(sb.get_seq_no(), sb_hash.clone());
        // finalize block by finality_hash
        self.finality_blocks(finality_hash, is_sync)?;
        self.save_finality()?;
        Ok(())
    }

    /// get last block sequence number
    fn get_last_seq_no(&self) -> u32 {
        self.current_block.block.block().read_info().unwrap().seq_no()
    }

    /// get last block info
    fn get_last_block_info(&self) -> NodeResult<BlkPrevInfo> {
        let info = &self.current_block.block.block().read_info()?;
        Ok(BlkPrevInfo::Block { prev: ExtBlkRef {
            end_lt: info.end_lt(),
            seq_no: info.seq_no(),
            root_hash: self.current_block.block_hash.clone(),//UInt256::from(self.current_block.block.block_hash().clone()),
            file_hash: self.current_block.file_hash.clone(),
        }})
    }

    /// get last shard bag
    fn get_last_shard_state(&self) -> Arc<ShardStateUnsplit> {
//warn!("LAST SHARD BAG {}", self.current_block.shard_bag.get_repr_hash_by_index(0).unwrap().to_hex_string()));
        Arc::clone(&self.current_block.shard_state)
    }
    /// find block by hash and return his secuence number (for sync)
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

            // remove from hashmaps all blocks with greather seq_no
            loop {
                let tmp = self.blocks_by_no.remove(&seq_no);
                if tmp.is_some() {
                    self.blocks_by_hash.remove(tmp.unwrap().block_hash());
                } else {
                    break;
                }
                seq_no += 1;
            }

            Ok(())
        } else {
            node_err!(NodeErrorKind::RoolbackBlockError)
        }
    }

    /// get raw signed block data - for synchronize
    fn get_raw_block_by_seqno(&self, seq_no: u32, vert_seq_no: u32 ) -> NodeResult<Vec<u8>> {
        let key = key_by_seqno(seq_no, vert_seq_no);
        if self.blocks_by_no.contains_key(&key) {
            /* TODO: wich case to use?
            return Ok(self.blocks_by_no.get(&key).unwrap().serialized_block.clone())
            TODO rewrite to
            return Ok(
                self.fn_storage.load_non_finalized_block_by_seq_no(key)?.serialized_block.clone()
            )*/
            return Ok(self
                        .stored_to_loaded(self.blocks_by_no.get(&key).unwrap().clone())?
                        .serialized_block.clone()
                    )
        }
        self.blocks_storage.raw_block(seq_no, vert_seq_no)
    }

    /// get number of last finalized shard
    fn get_last_finality_shard_hash(&self) -> NodeResult<(u64, UInt256)> {
        // TODO avoid serilization there
        let cell: Cell = self.last_finalized_block.shard_state.write_to_new_cell()?.into();
        
        Ok((self.last_finalized_block.seq_no, cell.repr_hash()))
    }

    /// reset block finality
    /// clean all maps, load last finalized data
    fn reset(&mut self) -> NodeResult<()> {
        {
            let mut prev_shard_state = Arc::new(ShardStateUnsplit::default());
            let prev_seq_no = self.current_block.seq_no - 1; // take previous seq_no, this func will not be called if current seq_no=0
            let prev_sb = self.blocks_by_no.get(&prev_seq_no); // get previous ShardState, 
            if prev_sb.is_some() {
                prev_shard_state = self.stored_to_loaded(prev_sb.unwrap().clone())?.shard_state.clone();
            }

            // update states in DB and restore previous state of accounts
            let res = self.reflect_block_in_db(
                self.current_block.block.block().clone(),
                prev_shard_state,
                false,
                BlockProcessingStatus::Refused,
                MessageProcessingStatus::Refused,
                TransactionProcessingStatus::Refused
            );

            if res.is_err() {
                warn!(target: "node", "reflect_block_in_db(Refused) error: {:?}", res.unwrap_err());
            }
        }
        self.current_block = self.last_finalized_block.clone();
        // remove files from disk
        for (hash, _sb) in self.blocks_by_hash.iter() {
            self.remove_form_finality_storage(&hash)?;
        }
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
            FinalityBlock::Stored(sb) => {
                sb.seq_no
            },
            FinalityBlock::Loaded(sb) => {
                sb.seq_no
            }
        }
    }

    pub fn block_hash(&self) -> &UInt256 {
        match self {
            FinalityBlock::Stored(sb) => {
                &sb.block_hash
            },
            FinalityBlock::Loaded(sb) => {
                &sb.block_hash
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShardBlockHash {
    seq_no: u64,
    block_hash: UInt256
}

impl ShardBlockHash {
    pub fn with_hash(seq_no: u64, hash: UInt256) -> Self {
        Self {
            seq_no,
            block_hash: hash
        }
    }
}

/// Structure for store one block and his ShardState
#[derive(Clone, Debug, PartialEq)]
pub struct ShardBlock {
    seq_no: u64,
    serialized_block: Vec<u8>,
    block_hash: UInt256,
    file_hash: UInt256,
    block: SignedBlock,
    shard_state: Arc<ShardStateUnsplit>,
}

impl Default for ShardBlock {
    fn default() -> Self {
        Self {
            seq_no: 0,
            serialized_block: Vec::new(),
            block_hash: UInt256::from([0;32]),
            file_hash: UInt256::from([0;32]),
            block: SignedBlock::with_block_and_key(
                Block::default(),
                &Keypair::from_bytes(&[0;64]).unwrap()
            ).unwrap(),
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
    pub fn block_hash(&self) -> &UInt256 {
        &self.block_hash
    }

    /// Create new instance of shard block with SignedBlock and new shard state
    pub fn with_block_and_state(
        sblock: SignedBlock,
        serialized_block: Option<Vec<u8>>,
        block_hash: Option<UInt256>,
        shard_state: Arc<ShardStateUnsplit>) -> Self 
    {
        let serialized_block = if serialized_block.is_some() { 
            serialized_block.unwrap() 
        } else {
            let mut buf = Vec::new();
            sblock.write_to(&mut buf).unwrap();
            buf
        };

        // Test-lite-client requires hash od unsigned block
        // TODO will to think, how to do better
        let mut block_data = vec![];
        let mut builder = BuilderData::new();
        sblock.block().write_to(&mut builder).unwrap(); // TODO process result
        let bag = BagOfCells::with_root(&builder.into());
        bag.write_to(&mut block_data, false).unwrap(); // TODO process result

        let mut hasher = Sha256::new();
		hasher.input(block_data.as_slice());
		let file_hash = UInt256::from(hasher.result().to_vec());

        Self {
            seq_no: key_by_seqno(sblock.block().read_info().unwrap().seq_no(), sblock.block().read_info().unwrap().vert_seq_no()),
            serialized_block,
            block_hash: if block_hash.is_some() { block_hash.unwrap() } else { sblock.block().hash().unwrap() },
            file_hash,
            block: sblock,
            shard_state,  
        }
    }

    /// serialize shard block (for save on disk)
    pub fn serialize(&self) -> NodeResult<Vec<u8>>{
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.seq_no.to_le_bytes());
        buf.extend_from_slice(&(self.serialized_block.len() as u32).to_le_bytes());
        buf.append(&mut self.serialized_block.clone());
        buf.append(&mut self.block_hash.as_slice().to_vec());
        buf.append(&mut self.file_hash.as_slice().to_vec());

        BagOfCells::with_root(&self.shard_state.write_to_new_cell()?.into())
            .write_to(&mut buf, false)?;

        let mut block_buf = Vec::new();
        self.block.write_to(&mut block_buf)?;
        buf.append(&mut block_buf);
        Ok(buf)
    }

    /// deserialize shard block
    pub fn deserialize<R: Read>(rdr: &mut R) -> NodeResult<Self> {
        let mut sb = ShardBlock::new();
        sb.seq_no = rdr.read_le_u64()?;
        let sb_len = rdr.read_le_u32()?;
        let mut sb_buf = vec![0; sb_len as usize];
        rdr.read(&mut sb_buf)?;
        sb.serialized_block = sb_buf;
        
        let hash = rdr.read_u256()?;
        sb.block_hash = UInt256::from(hash);
        
        let hash = rdr.read_u256()?;
        sb.file_hash = UInt256::from(hash);

        let mut shard_slice = deserialize_tree_of_cells(rdr)?.into();
        sb.shard_state.read_from(&mut shard_slice)?;

        sb.block = SignedBlock::read_from(rdr)?;

        Ok(sb)
    }
}

// runs 10 thread to generate 5000 accounts with 1 input and two ouput messages per every block
// finilizes block and return
#[allow(dead_code)]
pub(crate) fn generate_block_with_seq_no(shard_ident: ShardIdent, seq_no: u32, prev_info: BlkPrevInfo) -> Block {

    let block_builder = Arc::new(BlockBuilder::with_shard_ident(shard_ident, seq_no, prev_info,
         0, None, SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32));

    // start 10 thread for generate transaction
    for _ in 0..10 {
        let builder_clone = block_builder.clone();
        thread::spawn(move || {
            //println!("Thread write start.");
            let mut rng = rand::thread_rng();
            for _ in 0..5000 {
                let acc = AccountId::from_raw((0..32).map(|_| { rand::random::<u8>() }).collect::<Vec<u8>>(), 256);
                let mut transaction = Transaction::with_address_and_status(acc.clone(), AccountStatus::AccStateActive);
                
                let mut value = CurrencyCollection::default();
                value.grams = 10202u32.into();
                let mut imh = InternalMessageHeader::with_addresses (
                    MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    MsgAddressInt::with_standart(None, 0, 
                        AccountId::from_raw((0..32).map(|_| { rand::random::<u8>() }).collect::<Vec<u8>>(), 256)).unwrap(),
                    value
                );
                
                imh.ihr_fee = 10u32.into();
                imh.fwd_fee = 5u32.into();
                let mut inmsg1 = Arc::new(Message::with_int_header(imh));
                Arc::get_mut(&mut inmsg1).map(|m| *m.body_mut() = Some(SliceData::new(vec![0x21;120])));

                let inmsg_int = InMsg::immediatelly(
                    &MsgEnvelope::with_message_and_fee(&inmsg1, 9u32.into()).unwrap(),
                    &transaction,
                    11u32.into(),
                ).unwrap();

                let eimh = ExternalInboundMessageHeader {
                    src: MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80])).unwrap(),
                    dst: MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    import_fee: 10u32.into(),
                };

                let mut inmsg = Message::with_ext_in_header(eimh);
                *inmsg.body_mut() = Some(SliceData::new(vec![0x01;120]));

                transaction.write_in_msg(Some(&inmsg1)).unwrap();
                // inmsg
                let inmsg_ex = InMsg::external(&inmsg, &transaction).unwrap();
                
                // outmsgs
                let mut value = CurrencyCollection::default();
                value.grams = 10202u32.into();
                let mut imh = InternalMessageHeader::with_addresses (
                    MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    MsgAddressInt::with_standart(None, 0, AccountId::from_raw(vec![255;32], 256)).unwrap(),
                    value
                );
                
                imh.ihr_fee = 10u32.into();
                imh.fwd_fee = 5u32.into();
                let outmsg1 = Message::with_int_header(imh);

                let eomh = ExtOutMessageHeader::with_addresses (
                    MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
                    MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80])).unwrap()
                );
                
                let mut outmsg2 = Message::with_ext_out_header(eomh);
                *outmsg2.body_mut() = Some(SliceData::new(vec![0x02;120]));

                let tr_cell: Cell = transaction.write_to_new_cell().unwrap().into();

                let out_msg1 = OutMsg::new(
                    &MsgEnvelope::with_message_and_fee(&outmsg1, 9u32.into()).unwrap(),
                    tr_cell.clone()
                ).unwrap();

                let out_msg2 = OutMsg::external(&outmsg2, tr_cell).unwrap();

                let inmsg = Arc::new(if rng.gen() { inmsg_int} else { inmsg_ex });
                // builder can stop earler than writing threads it is not a problem here
                if !builder_clone.add_transaction(inmsg, vec![out_msg1, out_msg2]) {
                    break;
                }
                
                thread::sleep(Duration::from_millis(1)); // emulate timeout working TVM
            }
        });
    }

    thread::sleep(Duration::from_millis(10));
    
    let ss = ShardStateUnsplit::default();

    let (block, _count) = block_builder.finalize_block(&ss, &ss).unwrap();
    block
}
