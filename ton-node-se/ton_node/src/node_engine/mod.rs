use crate::ethcore_network::*;
use ed25519_dalek::{Keypair};
#[allow(deprecated)]
use super::error::*;
use parking_lot::{ Mutex };
use std;
use std::cell::{Cell as StdCell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::convert::From;
use std::clone::Clone;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{ Arc };
use std::thread::{JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ton_labs_assembler::compile_code;
use ton_block::{
    Account, BlkPrevInfo, Block, BlockProcessingStatus, CommonMsgInfo, 
    CurrencyCollection, ExtBlkRef, ExternalInboundMessageHeader, GetRepresentationHash, 
    Grams, InternalMessageHeader, Message, MessageProcessingStatus, 
    MsgAddressExt, MsgAddressInt, Serializable, Deserializable, ShardStateUnsplit, 
    ShardIdent, StateInit, Transaction, TransactionProcessingStatus, SignedBlock,
};
use ton_types::{ Cell, SliceData };
use ton_types::types::{ UInt256, AccountId, ByteOrderRead };

pub mod block_builder;
pub use self::block_builder::*;

pub mod file_based_storage;
use self::file_based_storage::*;

pub mod messages;
pub use self::messages::*;

pub mod new_block_applier;
use self::new_block_applier::*;

pub mod blocks_finality;
use self::blocks_finality::*;

pub mod ton_node_engine;
use self::ton_node_engine::*;

pub mod ton_node_handlers;
pub mod adnl_server_handler;

pub mod config;
use self::config::*;

pub mod routing_table;
use self::routing_table::*;

use std::{io::Read, thread};

pub struct StubReceiver {
    stop_tx: Option<mpsc::Sender<bool>>,
    join_handle: Option<JoinHandle<()>>,
    workchain_id: i8,
    block_seqno: u32,
    timeout: u64
}

lazy_static! {
    static ref ACCOUNTS: Mutex<Vec<AccountId>> = Mutex::new(vec![]);

    static ref SUPER_ACCOUNT_ID: AccountId = AccountId::from([0;32]);
}

static GIVER_GRAMS: u128 = 5_000_000_000_000_000_000;
static ACCOUNTS_COUNT: u8 = 255;

impl MessagesReceiver for StubReceiver {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {

        if self.block_seqno == 1 {
            let queue_with_retry = |mut message| {
                while let Err(msg) = queue.queue(message) {
                    message = msg;
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
            };

            //if it's block 1 then create giver contract: 
            let (giver_msg, giver_addr) = Self::create_giver_deploy_message(self.workchain_id, GIVER_ABI1_DEPLOY_MSG);
            let transfer_msg = Self::create_transfer_message(
                self.workchain_id,
                giver_addr.clone(),
                giver_addr.clone(),
                GIVER_GRAMS,
                0
            );
            queue_with_retry(QueuedMessage::with_message(transfer_msg)?);
            queue_with_retry(QueuedMessage::with_message(giver_msg)?);
        }

        if self.timeout == 0 {
            return Ok(());
        }

        for acc in 0..ACCOUNTS_COUNT {
            ACCOUNTS.lock().push(AccountId::from([acc + 1;32]));
        }

        if self.join_handle.is_none() {
            let (tx, rx) = mpsc::channel();
            self.stop_tx = Some(tx);
            let mut log_time_gen = LogicalTimeGenerator::with_init_value(0);

            let workchain_id = self.workchain_id;
            let timeout = self.timeout;
            self.join_handle = Some(std::thread::spawn(move || {
                loop {
                    if let Some(msg) = Self::try_receive_message(workchain_id, &mut log_time_gen) {
                        let _res = queue.queue(QueuedMessage::with_message(msg).unwrap());
                    }
                    if rx.try_recv().is_ok() {
                        println!("append message loop break");
                        break;
                    }
                    thread::sleep(Duration::from_micros(timeout));
                }
                // Creation of special account zero to give money for new accounts
                queue.queue(QueuedMessage::with_message(Self::create_transfer_message(
                    workchain_id, 
                    SUPER_ACCOUNT_ID.clone(),
                    SUPER_ACCOUNT_ID.clone(), 
                    1_000_000, 
                    log_time_gen.get_current_time()
                )).unwrap()).unwrap();
                queue.queue(QueuedMessage::with_message(Self::create_account_message_with_code(
                    workchain_id, 
                    SUPER_ACCOUNT_ID.clone()
                )).unwrap()).unwrap();
            }));

        }
        Ok(())
    }
}

const GIVER_ABI1_DEPLOY_MSG: &'static str = "B5EE9C724104260100000005D500035F8801082511DA76ABB39B5F500D00FE05415C182D354BDBFD114F134C904852EAD52811828900274000002DCAE4C7E95007020100000101C0030203CF2006040101DE050003D0200041D800000000000000000000000000000000000000000000000000000000000000040228FF0020C001F4A4205892F4A0E18AED535830F4A01408010AF4A420F4A1090203CD40110A0201CE0E0B0201200D0C00070C36CC20001D3C06486F3CAF08485C1C7C0297C0A0020120100F00012000D53FBEC0591958DBD91957D859191C883E900CBE90881BC4081CAE885CEEAC7CB81F485BC45BBCB81F721D33C2C0889BC4B3C281C89BC4C89CAEA588DBC4C8B38CA7C860404008B5D26873D00C8808B38CB8BFBF00591958DBD91957D859191C8C08727409555057C176CC200201201312002BA4FF7D80B3B2BA2FB130B630B731B2FC13B7886D984000A5A57F7D80B0B1AFBA3930B739B332B9643967A01167850038E7A07C14678B12678B11FD0138E7A0387D01387D01402067A07C11E7858FB967A01064917D807F7F80B0B1AFBA3930B739B332B92FB2B7322F82C00201201B1501E0FFFEFD016D61696E5F65787465726E616C218E59FEFC016765745F7372635F6164647220D020D3003270BD8E1AFEFD016765745F7372635F616464723070C8C9D055115F02DB30E02072D7213120D3003221FA4033FEFD016765745F7372635F6164647231212155315F04DB30D831211601F88E75FEFE016765745F6D73675F7075626B657920C7028E16FEFF016765745F6D73675F7075626B6579317031DB30E0D520C7018E17FEFF016765745F6D73675F7075626B657932703131DB30E020810200D721D70BFF22F9012222F910F2A8FEFF016765745F6D73675F7075626B65793320035F03DB30D822C702B31701CC9422D43133DE2422228E38FEF90173746F72655F7369676F00216F8C226F8C236F8CED47216F8CED44D0F4056F8C20ED57FEFD0173746F72655F7369675F656E645F05D822C7018E13FEFC016D73675F69735F656D7074795F06DB30E022D31F3423D33F35201801768E80D88E2FFEFE016D61696E5F65787465726E616C32242255715F08F14001FEFE016D61696E5F65787465726E616C335F08DB30E0807CF2F05F081901FEFEFB017265706C61795F70726F74707070ED44D020F404323420810080D7459A20D33F323320D33F32329682081B774032E22225B925F8238103E8A824A0B9B08E29C82401F40025CF0B3F22CF0B3F21CF1620C9ED54FEFC017265706C61795F70726F74327F065F06DB30E0FEFC017265706C61795F70726F743370055F051A0004DB300201201D1C000FBCE3F7103986D9840201201F1E0089BB25CCBC9F800F020D33F30F022FEFC017075736870646337746F6334ED44D0F401C8ED476F1201F40021CF1620C9ED54FEFD017075736870646337746F6334305F02DB30802012023200109B8890027502101FEFEFD01636F6E7374725F70726F745F30707082081B7740ED44D020F404323420810080D7458E1420D23F323320D23F32322071D74594807BF2F0DEDEC82401F40023CF0B3F22CF0B3F71CF4121CF1620C9ED54FEFD01636F6E7374725F70726F745F315F05F80030F021FEFC017075736870646337746F6334ED44D0F401C8220044ED476F1201F40021CF1620C9ED54FEFD017075736870646337746F6334305F02DB3001E2DCFEFD016D61696E5F696E7465726E616C218E59FEFC016765745F7372635F6164647220D020D3003270BD8E1AFEFD016765745F7372635F616464723070C8C9D055115F02DB30E02072D7213120D3003221FA4033FEFD016765745F7372635F6164647231212155315F04DB30D82421702401EA8E38FEF90173746F72655F7369676F00216F8C226F8C236F8CED47216F8CED44D0F4056F8C20ED57FEFD0173746F72655F7369675F656E645F05D822C7008E1C2170BA8E122282105C7EE20755515F06F140015F06DB30E05F06DB30E0FEFE016D61696E5F696E7465726E616C3122D31F342271BA2500369E20802355615F07F140015F07DB30E0232155615F07F140015F07F6082CD4";

#[allow(dead_code)]
impl StubReceiver {

    pub fn with_params(workchain_id: i8, block_seqno: u32, timeout: u64) -> Self {
        StubReceiver {
            stop_tx: None,            
            join_handle: None,
            workchain_id,
            block_seqno,
            timeout
        }
    }

    pub fn stop(&mut self) {
        if self.join_handle.is_some() {
            if let Some(ref stop_tx) = self.stop_tx {
                stop_tx.send(true).unwrap();
                self.join_handle.take().unwrap().join().unwrap();                
            }
        }
    }

    fn create_account_message_with_code(workchain_id: i8, account_id: AccountId) -> Message {
        let code = "
        ; s0 - function selector
        ; s1 - body slice
        IFNOTRET
        ACCEPT
        DUP
        SEMPTY
        IFRET
        ACCEPT
        BLOCKLT
        LTIME
        INC         ; increase logical time by 1
        PUSH s2     ; body to top
        PUSHINT 96  ; internal header in body, cut unixtime and lt
        SDSKIPLAST
        NEWC
        STSLICE
        STU 64         ; store tr lt
        STU 32         ; store unixtime
        STSLICECONST 0 ; no init
        STSLICECONST 0 ; body (Either X)
        ENDC
        PUSHINT 0
        SENDRAWMSG
        ";    
        Self::create_account_message(workchain_id, account_id, code, SliceData::new_empty().into_cell(), None)
    }

    fn create_giver_deploy_message(workchain_id: i8, giver_boc: &str) -> (Message, AccountId) {
        let data = hex::decode(giver_boc).unwrap();
        let mut msg = Message::construct_from_bytes(&data).unwrap();
        if let CommonMsgInfo::ExtInMsgInfo(ref mut header) = msg.header_mut() {
            match header.dst {
                MsgAddressInt::AddrStd(ref mut addr) => addr.workchain_id = workchain_id,
                _ => panic!("Giver deploy message has invalid destination address")
            }
        }

        let address = msg.int_dst_account_id().unwrap();

        (msg, address)
    }


    // create external message with init field, so-called "constructor message"
    pub fn create_account_message(
        workchain_id: i8, 
        account_id: AccountId, 
        code: &str, 
        data: Cell, 
        body: Option<SliceData>
    ) -> Message {
        
        let code_cell = compile_code(code).unwrap().into_cell();
            
        let mut msg = Message::with_ext_in_header(
            ExternalInboundMessageHeader {
                src: MsgAddressExt::default(),
                dst: MsgAddressInt::with_standart(None, workchain_id, account_id.clone()).unwrap(),
                import_fee: Grams::zero(),
            }
        );

        let mut state_init = StateInit::default();        
        state_init.set_code(code_cell);   
        state_init.set_data(data);
        *msg.state_init_mut() = Some(state_init);

        *msg.body_mut() = body;

        msg
    }
    // create transfer funds message for initialize balance
    pub fn create_transfer_message(
        workchain_id: i8, 
        src: AccountId, 
        dst: AccountId, 
        value: u128, 
        lt: u64
    ) -> Message {
        
        let mut balance = CurrencyCollection::default();
        balance.grams = value.into();

        let mut msg = Message::with_int_header(
            InternalMessageHeader::with_addresses_and_bounce(
                MsgAddressInt::with_standart(None, workchain_id, src).unwrap(),
                MsgAddressInt::with_standart(None, workchain_id, dst).unwrap(),
                balance,
                false,
            )
        );

        msg.set_at_and_lt(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32, lt);
        msg
    }

    // Create message "from wallet" to transfer some funds 
    // from one account to another
    pub fn create_external_transfer_funds_message(
        workchain_id: i8, 
        src: AccountId, 
        dst: AccountId, 
        value: u128, 
        _lt: u64
    ) -> Message {

        let mut msg = Message::with_ext_in_header(
            ExternalInboundMessageHeader {
                src: MsgAddressExt::default(),
                dst: MsgAddressInt::with_standart(None, workchain_id, src.clone()).unwrap(),
                import_fee: Grams::zero(),
            }
        );

        *msg.body_mut() = Some(Self::create_transfer_int_header(workchain_id, src, dst, value)
            .write_to_new_cell()
            .unwrap()
            .into()
        );

        msg
    }

    pub fn create_transfer_int_header(
        workchain_id: i8, 
        src: AccountId, 
        dest: AccountId, 
        value: u128
    ) -> InternalMessageHeader {
        let msg = Self::create_transfer_message(workchain_id, src, dest, value, 0);
        match msg.withdraw_header() {
            CommonMsgInfo::IntMsgInfo(int_hdr) => int_hdr,
            _ => panic!("must be internal message header"),
        }
    }

    fn try_receive_message(workchain_id: i8, log_time_gen: &mut LogicalTimeGenerator) -> Option<Message> {
        let time = log_time_gen.get_next_time();
        Some(match time - 1 {
            x if x < (ACCOUNTS_COUNT as u64) => {
                Self::create_transfer_message(
                    workchain_id, 
                    SUPER_ACCOUNT_ID.clone(),
                    ACCOUNTS.lock()[x as usize].clone(), 
                    1000000, 
                    log_time_gen.get_current_time()
                )
            },
            x if x >= (ACCOUNTS_COUNT as u64) && x < (ACCOUNTS_COUNT as u64)*2 => {
                Self::create_transfer_message(
                    workchain_id, 
                    SUPER_ACCOUNT_ID.clone(),
                    ACCOUNTS.lock()[(x - ACCOUNTS_COUNT as u64) as usize].clone(), 
                    1000000, 
                    log_time_gen.get_current_time()
                )
            }
            x if x >= (ACCOUNTS_COUNT as u64)*2 && x < (ACCOUNTS_COUNT as u64)*3 => {
                let index = (x - (ACCOUNTS_COUNT as u64)*2) as usize;
                Self::create_account_message_with_code(workchain_id, ACCOUNTS.lock()[index].clone())
            }
            x => { // send funds from 1 to 2, after from 2 to 3 and etc
                let acc_src = (x%ACCOUNTS_COUNT as u64) as usize;
                let acc_dst = (acc_src + 1) % ACCOUNTS_COUNT as usize;
                let src_acc_id = ACCOUNTS.lock()[acc_src].clone();
                let dst_acc_id = ACCOUNTS.lock()[acc_dst].clone();
                Self::create_external_transfer_funds_message(
                    workchain_id, 
                    src_acc_id,
                    dst_acc_id, 
                    rand::random::<u8>() as u128, 
                    log_time_gen.get_current_time()
                )
            }
        })
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
    pub fn with_params(seq_no: u64, lt: u64, hash: UInt256) -> Self {
        Self {
            seq_no,
            lt,
            hash,
        }
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
        Ok(ShardStateInfo {seq_no, lt, hash})
    }
}

/// Trait for shard state storage
pub trait ShardStateStorage {
    fn shard_state(&self) -> NodeResult<ShardStateUnsplit>;
    fn shard_bag(&self) -> NodeResult<Cell>;
    fn save_shard_state(&self, shard_state: &ShardStateUnsplit) -> NodeResult<()>;
    fn serialized_shardstate(&self) -> NodeResult<Vec<u8>>;
    fn save_serialized_shardstate(&self, data: Vec<u8>) -> NodeResult<()>;
    fn save_serialized_shardstate_ex(&self, shard_state: &ShardStateUnsplit, 
            shard_data: Option<Vec<u8>>, shard_hash: &UInt256,
            shard_state_info: ShardStateInfo) -> NodeResult<()>;
}

// Trait for blocks storage (key-value)
pub trait BlocksStorage {
    fn block(&self, seq_no: u32, vert_seq_no: u32 ) -> NodeResult<SignedBlock>;
    fn raw_block(&self, seq_no: u32, vert_seq_no: u32 ) -> NodeResult<Vec<u8>>;
    fn save_block(&self, block: &SignedBlock) -> NodeResult<()>;
    fn save_raw_block(&self, block: &SignedBlock, block_data: Option<&Vec<u8>>) -> NodeResult<()>;
}

/// Trait for transactions storage (this storage have to support difficult queries)
pub trait TransactionsStorage {
    fn save_transaction(&self, tr: Arc<Transaction>) -> NodeResult<()>;
    fn find_by_lt(&self, _lt: u64, _acc_id: &AccountId) -> NodeResult<Option<Transaction>> {unimplemented!()}
}

/// Trait for save finality states blockchain
pub trait BlockFinality {
    fn finalize_without_new_block(&mut self, finality_hash: Vec<UInt256>) -> NodeResult<()>;

    fn put_block_with_info(&mut self,
            sblock: SignedBlock,
            sblock_data:Option<Vec<u8>>,
            block_hash: Option<UInt256>,
            shard_state: Arc<ShardStateUnsplit>,
            finality_hashes: Vec<UInt256>,
            is_sync: bool,
        ) -> NodeResult<()>;
    
    fn get_last_seq_no(&self) -> u32;

    fn get_last_block_info(&self) -> NodeResult<BlkPrevInfo>;

    fn get_last_shard_state(&self) -> Arc<ShardStateUnsplit>;

    fn find_block_by_hash(&self, hash: &UInt256) -> u64;

    fn rollback_to(&mut self, hash: &UInt256) -> NodeResult<()>;
    
    fn get_raw_block_by_seqno(&self, seq_no: u32, vert_seq_no: u32 ) -> NodeResult<Vec<u8>>;

    fn get_last_finality_shard_hash(&self) -> NodeResult<(u64, UInt256)>;

    fn reset(&mut self) -> NodeResult<()>;
}

pub trait MessagesReceiver: Send {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()>;
}

pub trait DocumentsDb: Send + Sync {
    fn put_account(&self, acc: Account) -> NodeResult<()>;
    fn put_deleted_account(&self, workchain_id: i32, account_id: AccountId) -> NodeResult<()>;
    fn put_block(&self, block: Block, status: BlockProcessingStatus) -> NodeResult<()>;
    fn put_message(&self, msg: Message, status: MessageProcessingStatus,
        transaction_id: Option<UInt256>, block_id: Option<UInt256>) -> NodeResult<()>;
    fn put_transaction(&self, tr: Transaction, status: TransactionProcessingStatus, 
        block_id: Option<UInt256>, workchain_id: i32) -> NodeResult<()>;
    fn has_delivery_problems(&self) -> bool;
}

pub struct DocumentsDbMock;
impl DocumentsDb for DocumentsDbMock {
    fn put_account(&self, _: Account) -> NodeResult<()> { 
        Ok(()) 
    }

    fn put_deleted_account(&self, _: i32, _: AccountId) -> NodeResult<()> {
        Ok(())
    }

    fn put_block(&self, _: Block, _: BlockProcessingStatus) -> NodeResult<()> {
        Ok(()) 
    }

    fn put_message(&self, _: Message, _: MessageProcessingStatus, _: Option<UInt256>, _: Option<UInt256>) 
        -> NodeResult<()> {
        Ok(())
    }

    fn put_transaction(&self, _: Transaction, _: TransactionProcessingStatus, 
        _: Option<UInt256>, _: i32) -> NodeResult<()> {
        Ok(()) 
    }

    fn has_delivery_problems(&self) -> bool {
        false
    }
}

struct TestStorage {
    shard_ident: ShardIdent,
    shard_state: StdCell<ShardStateUnsplit>,
    blocks: RefCell<HashMap<UInt256, SignedBlock>>,
    transactions: RefCell<Vec<Transaction>>,
    finality_by_hash: RefCell<HashMap<UInt256, Vec<u8>>>,
    finality_by_no: RefCell<HashMap<u64, Vec<u8>>>,
    finality_by_str: RefCell<HashMap<String, Vec<u8>>>,
}

impl TestStorage {    
    #[allow(dead_code)]
    pub fn new(shard_ident: ShardIdent) -> Self {
        TestStorage {
            shard_ident,
            shard_state: StdCell::new(ShardStateUnsplit::default()),
            blocks: RefCell::new(HashMap::new()),
            transactions: RefCell::new(Vec::new()),
            finality_by_hash: RefCell::new(HashMap::new()),
            finality_by_no: RefCell::new(HashMap::new()),
            finality_by_str: RefCell::new(HashMap::new()),
        }
    }

    ///
    /// Get hash-identifier form shard ident and sequence numbers
    /// 
    fn get_hash_from_ident_and_seq(shard_ident: &ShardIdent, seq_no: u32, vert_seq_no: u32) -> UInt256 {
        let mut hash = vec![];
        // TODO: check here
        hash.extend_from_slice(&(shard_ident.shard_prefix_with_tag()).to_be_bytes());
        hash.extend_from_slice(&(seq_no).to_be_bytes());
        hash.extend_from_slice(&(vert_seq_no).to_be_bytes());
        UInt256::from(hash)
    }
}

impl ShardStateStorage for TestStorage {
    fn shard_state( &self ) -> NodeResult<ShardStateUnsplit> {
        let ss = self.shard_state.take();
        self.shard_state.set(ss.clone());
        Ok(ss)
    }
    fn shard_bag( &self ) -> NodeResult<Cell> {
        let ss = self.shard_state.take();
        self.shard_state.set(ss.clone());
        Ok(Cell::default())
    }
    fn save_shard_state(&self, shard_state: &ShardStateUnsplit) -> NodeResult<()> {
        self.shard_state.set(shard_state.clone());
        Ok(())
    }

    fn serialized_shardstate(&self) -> NodeResult<Vec<u8>>{
        Ok(vec![])
    }
    fn save_serialized_shardstate(&self, _data: Vec<u8>) -> NodeResult<()>{
        Ok(())
    }
    fn save_serialized_shardstate_ex(&self, _shard_state: &ShardStateUnsplit, 
            _shard_data: Option<Vec<u8>>, _shard_hash: &UInt256,
            _shard_state_info: ShardStateInfo) -> NodeResult<()>{
        Ok(())
    }
}

impl BlocksStorage for TestStorage {

    ///
    /// Get block from memory storage by ID
    /// 
    fn block(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<SignedBlock>{
        let hash = Self::get_hash_from_ident_and_seq(&self.shard_ident, seq_no, vert_seq_no);
        match self.blocks.borrow().get(&hash) {
            Some(b) => Ok(b.clone()),
            _ => Err(NodeError::from_kind(NodeErrorKind::NotFound))
        }
    }

    fn raw_block(&self, _seq_no: u32, _vert_seq_no: u32 ) -> NodeResult<Vec<u8>>{
        Ok(vec![])
    }

    ///
    /// Save block to memory storage
    /// 
    fn save_block(&self, block: &SignedBlock) -> NodeResult<()>{
        let info = block.block().read_info()?;
        let hash = Self::get_hash_from_ident_and_seq(&info.shard(), info.seq_no(), info.vert_seq_no());
        self.blocks.try_borrow_mut().unwrap().insert(UInt256::from(hash), block.clone());
        Ok(())
    }

    fn save_raw_block(&self, _block: &SignedBlock, _block_data: Option<&Vec<u8>>) -> NodeResult<()> {
        info!(target: "node", "save block with seq_no: {}", _block.block().read_info()?.seq_no());
        Ok(())
    }
}

impl TransactionsStorage for TestStorage {
    fn save_transaction(&self, tr: Arc<Transaction>) -> NodeResult<()>{
        self.transactions.borrow_mut().push((*tr).clone());
        Ok(())
    }
    fn find_by_lt(&self, lt: u64, _acc_id: &AccountId) -> NodeResult<Option<Transaction>> {
        for tr in self.transactions.borrow().iter() {
            if tr.logical_time() == lt {
                return Ok(Some(tr.clone()))
            }
        }
        Ok(None)
    }
}

impl FinalityStorage for TestStorage {
    fn save_non_finalized_block(&self, hash: UInt256, seq_no: u64, data: Vec<u8>) -> NodeResult<()> {
        println!("save block {:?}", hash);
        self.finality_by_hash.try_borrow_mut().unwrap().insert(hash, data.clone());
        self.finality_by_no.try_borrow_mut().unwrap().insert(seq_no, data);
        Ok(())
    }
    fn load_non_finalized_block_by_seq_no(&self, seq_no: u64) -> NodeResult<Vec<u8>> {
        println!("load block {:?}", seq_no);
        if self.finality_by_no.borrow().contains_key(&seq_no) {
            Ok(self.finality_by_no.try_borrow_mut().unwrap().get(&seq_no).unwrap().clone())
        } else {
            Err(NodeError::from_kind(NodeErrorKind::NotFound))
        }
    }
    fn load_non_finalized_block_by_hash(&self, hash: UInt256) -> NodeResult<Vec<u8>> {
        println!("load block {:?}", hash);
        if self.finality_by_hash.borrow().contains_key(&hash) {
            Ok(self.finality_by_hash.try_borrow_mut().unwrap().get(&hash).unwrap().clone())
        } else {
            Err(NodeError::from_kind(NodeErrorKind::NotFound))
        }
    }
    fn remove_form_finality_storage(&self, hash: UInt256) -> NodeResult<()> {
        println!("remove block {:?}", hash);
        self.finality_by_hash.try_borrow_mut().unwrap().remove(&hash).unwrap();
        Ok(())
    }
    fn save_custom_finality_info(&self, key: String, data: Vec<u8>) -> NodeResult<()> {
        println!("save custom {}", key);
        self.finality_by_str.try_borrow_mut().unwrap().insert(key, data);
        Ok(())
    }
    fn load_custom_finality_info(&self, key: String) -> NodeResult<Vec<u8>> {
        println!("load custom {}", key);
        if self.finality_by_str.borrow().contains_key(&key) {
            Ok(self.finality_by_str.try_borrow_mut().unwrap().remove(&key).unwrap())
        } else {
            Err(NodeError::from_kind(NodeErrorKind::NotFound))
        }    
    }
}

pub fn hexdump(d: &[u8]) {
    let mut str = String::new();
    for i in 0..d.len() {
        str.push_str(&format!("{:02x}{}", d[i], if (i + 1) % 16 == 0 { '\n' } else { ' ' }));
    }

    debug!(target: "node", "{}", str);
}

///
/// Struct LogicalTime Generator
///
pub struct LogicalTimeGenerator {
    current: Arc<Mutex<u64>>
}

impl Default for LogicalTimeGenerator {
    fn default() -> Self {
        LogicalTimeGenerator {
            current: Arc::new(Mutex::new(0))
        }
    }
}

///
/// Implementation of Logical Time Generator
///
impl LogicalTimeGenerator {

    ///
    /// Initialize new instance with current value
    ///
    pub fn with_init_value(current: u64) -> Self {
        LogicalTimeGenerator {
            current : Arc::new(Mutex::new(current))
        }
    }

    ///
    /// Get next value of logical time
    ///
    pub fn get_next_time(&mut self) -> u64 {
        let mut current = self.current.lock();
        *current += 1;
        *current
    }

    ///
    /// Get current value of logical time
    ///
    pub fn get_current_time(&self) -> u64 {
        let current = self.current.lock();
        *current
    }
}
