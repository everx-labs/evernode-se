use super::*;
#[allow(deprecated)]
use crate::error::NodeResult;
use crate::node_engine::stub_receiver::StubReceiver;
use crate::node_engine::DocumentsDb;
use node_engine::documents_db_mock::DocumentsDbMock;
use parking_lot::Mutex;
use std::cmp::Ordering;
use std::io::ErrorKind;
use std::sync::{
    atomic::Ordering as AtomicOrdering,
    atomic::AtomicU32,
    Arc,
};
use std::time::Duration;
use ton_executor::BlockchainConfig;

#[cfg(test)]
#[path = "../../../tonos-se-tests/unit/test_ton_node_engine.rs"]
mod tests;

type Storage = FileBasedStorage;
type ArcBlockFinality = Arc<Mutex<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;
type ArcMsgProcessor = Arc<Mutex<MessagesProcessor<Storage>>>;
type BlockApplier =
    Mutex<NewBlockApplier<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;

// TODO do interval and validator field of TonNodeEngine
// and read from config
pub const INTERVAL: usize = 2;
pub const VALIDATORS: usize = 7;

pub const GEN_BLOCK_TIMEOUT: u64 = 400;
//800;
pub const GEN_BLOCK_TIMER: u64 = 20;
pub const RESET_SYNC_LIMIT: usize = 5;

pub struct EngineLiveProperties {
    pub time_delta: Arc<AtomicU32>,
}

impl EngineLiveProperties {
    fn new() -> Self {
        Self {
            time_delta: Arc::new(AtomicU32::new(0)),
        }
    }

    fn get_time_delta(&self) -> u32 {
        self.time_delta.load(AtomicOrdering::Relaxed)
    }

    fn set_time_delta(&self, value: u32) {
        self.time_delta.store(value, AtomicOrdering::Relaxed)
    }

    fn increment_time(&self, delta: u32) {
        self.set_time_delta(self.get_time_delta() + delta)
    }
}

struct EngineLiveControl {
    properties: Arc<EngineLiveProperties>,
}

impl EngineLiveControl {
    fn new(properties: Arc<EngineLiveProperties>) -> Self {
        Self { properties }
    }
}

impl LiveControl for EngineLiveControl {
    fn increase_time(&self, delta: u32) -> NodeResult<()> {
        self.properties.increment_time(delta);
        Ok(())
    }
}

/// It is top level struct provided node functionality related to transactions processing.
/// Initialises instances of: all messages receivers, InMessagesQueue, MessagesProcessor.
pub struct TonNodeEngine {
    shard_ident: ShardIdent,

    live_properties: Arc<EngineLiveProperties>,
    receivers: Vec<Mutex<Box<dyn MessagesReceiver>>>,
    live_control_receiver: Box<dyn LiveControlReceiver>,

    interval: usize,
    get_block_timout: u64,
    gen_block_timer: u64,
    reset_sync_limit: usize,

    pub msg_processor: ArcMsgProcessor,
    pub finalizer: ArcBlockFinality,
    pub block_applier: BlockApplier,
    pub message_queue: Arc<InMessagesQueue>,
    pub incoming_blocks: IncomingBlocksCache,

    pub last_step: AtomicU32,

    #[cfg(test)]
    pub test_counter_in: Arc<Mutex<u32>>,
    #[cfg(test)]
    pub test_counter_out: Arc<Mutex<u32>>,

    pub(crate) private_key: Keypair,
}

impl TonNodeEngine {
    pub fn start(self: Arc<Self>) -> NodeResult<()> {
        for recv in self.receivers.iter() {
            recv.lock().run(Arc::clone(&self.message_queue))?;
        }

        let live_control = EngineLiveControl::new(self.live_properties.clone());
        self.live_control_receiver.run(Box::new(live_control))?;

        let node = self.clone();
        thread::spawn(move || {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32
                + self.live_properties.get_time_delta();
            loop {
                thread::sleep(Duration::from_secs(1));
                match node.prepare_block(timestamp) {
                    Ok(Some(_block)) => {
                        trace!(target: "node", "block generated successfully");
                    }
                    Ok(None) => {
                        trace!(target: "node", "block was not generated successfully");
                    }
                    Err(err) => {
                        warn!(target: "node", "failed block generation: {}", err);
                    }
                }
            }
        });

        Ok(())
    }

    pub fn stop(self: Arc<Self>) -> NodeResult<()> {
        info!(target: "node","TONNodeEngine stopped.");
        Ok(())
    }

    /// Construct new engine for selected shard
    /// with given time to generate block candidate
    pub fn with_params(
        shard: ShardIdent,
        _local: bool,
        _port: u16,
        _node_index: u8,
        _poa_validators: u16,
        _poa_interval: u16,
        private_key: Keypair,
        _public_keys: Vec<ed25519_dalek::PublicKey>,
        _boot_list: Vec<String>,
        receivers: Vec<Box<dyn MessagesReceiver>>,
        live_control_receiver: Box<dyn LiveControlReceiver>,
        blockchain_config: BlockchainConfig,
        documents_db: Option<Box<dyn DocumentsDb>>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        let documents_db: Arc<dyn DocumentsDb> = match documents_db {
            Some(documents_db) => Arc::from(documents_db),
            None => Arc::new(DocumentsDbMock)
        };
        let message_queue = Arc::new(InMessagesQueue::with_db(
            shard.clone(),
            10000,
            documents_db.clone(),
        ));

        let storage = Arc::new(Storage::with_path(shard.clone(), storage_path.clone())?);
        let block_finality = Arc::new(Mutex::new(OrdinaryBlockFinality::with_params(
            shard.clone(),
            storage_path,
            storage.clone(),
            storage.clone(),
            storage.clone(),
            storage.clone(),
            Some(documents_db.clone()),
            Vec::new(),
        )));
        match block_finality.lock().load() {
            Ok(_) => {
                info!(target: "node", "load block finality successfully");
            }
            Err(NodeError::Io(err)) => {
                    if err.kind() != ErrorKind::NotFound {
                        return Err(NodeError::Io(err));
                    }
                }
            Err(err) => {
                return Err(err);
            }
        }

        let live_properties = Arc::new(EngineLiveProperties::new());

        message_queue.set_ready(true);

        let mut receivers = receivers
            .into_iter()
            .map(|r| Mutex::new(r))
            .collect::<Vec<_>>();

        //TODO: remove on production or use only for tests
        receivers.push(Mutex::new(Box::new(StubReceiver::with_params(
            shard.workchain_id() as i8,
            block_finality.lock().get_last_seq_no(),
            0,
        ))));

        Ok(TonNodeEngine {
            shard_ident: shard.clone(),
            receivers,
            live_properties,
            live_control_receiver,
            last_step: AtomicU32::new(100500),

            interval: 1,
            get_block_timout: GEN_BLOCK_TIMEOUT,
            gen_block_timer: GEN_BLOCK_TIMER,
            reset_sync_limit: RESET_SYNC_LIMIT,

            msg_processor: Arc::new(Mutex::new(MessagesProcessor::with_params(
                message_queue.clone(),
                storage.clone(),
                shard.clone(),
                blockchain_config,
            ))),
            finalizer: block_finality.clone(),
            block_applier: Mutex::new(NewBlockApplier::with_params(
                block_finality,
                documents_db,
            )),
            message_queue,
            incoming_blocks: IncomingBlocksCache::new(),
            #[cfg(test)]
            test_counter_out: Arc::new(Mutex::new(0)),
            #[cfg(test)]
            test_counter_in: Arc::new(Mutex::new(0)),

            private_key,
        })
    }

    pub fn interval(&self) -> usize {
        self.interval
    }

    pub fn push_message(&self, mut message: QueuedMessage, warning: &str, micros: u64) {
        while let Err(msg) = self.message_queue.queue(message) {
            message = msg;
            warn!(target: "node", "{}", warning);
            thread::sleep(Duration::from_micros(micros));
        }
    }

    pub fn gen_block_timeout(&self) -> u64 {
        self.get_block_timout
    }

    pub fn gen_block_timer(&self) -> u64 {
        self.gen_block_timer
    }

    pub fn reset_sync_limit(&self) -> usize {
        self.reset_sync_limit.clone()
    }

    /// Getter for current shard identifier
    pub fn current_shard_id(&self) -> &ShardIdent {
        &self.shard_ident
    }
}

pub fn get_config_params(json: &str) -> (NodeConfig, Vec<ed25519_dalek::PublicKey>) {
    match NodeConfig::parse(json) {
        Ok(config) => match config.import_keys() {
            Ok(keys) => (config, keys),
            Err(err) => {
                warn!(target: "node", "{}", err);
                panic!("{} / {}", err, json)
            }
        },
        Err(err) => {
            warn!(target: "node", "Error parsing configuration file. {}", err);
            panic!("Error parsing configuration file. {}", err)
        }
    }
}

#[derive(Eq, Clone, Debug)]
pub struct FinalityBlockInfo {
    pub block: SignedBlock,
    pub block_data: Option<Vec<u8>>,
    pub hashes: Vec<UInt256>,
}

impl FinalityBlockInfo {
    pub fn with_block(
        block: SignedBlock,
        block_data: Option<Vec<u8>>,
        hashes: Vec<UInt256>,
    ) -> Self {
        FinalityBlockInfo {
            block,
            block_data,
            hashes,
        }
    }
}

impl Ord for FinalityBlockInfo {
    fn cmp(&self, other: &FinalityBlockInfo) -> Ordering {
        self.block
            .block()
            .read_info()
            .unwrap()
            .seq_no()
            .cmp(&other.block.block().read_info().unwrap().seq_no())
    }
}

impl PartialOrd for FinalityBlockInfo {
    fn partial_cmp(&self, other: &FinalityBlockInfo) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for FinalityBlockInfo {
    fn eq(&self, other: &FinalityBlockInfo) -> bool {
        self.block.block().read_info().unwrap().seq_no()
            == other.block.block().read_info().unwrap().seq_no()
    }
}

pub struct IncomingBlocksCache {
    blocks: Arc<Mutex<Vec<FinalityBlockInfo>>>,
}

impl IncomingBlocksCache {
    /// create new instance of TemporaryBlocks
    fn new() -> Self {
        Self {
            blocks: Arc::new(Mutex::new(vec![])),
        }
    }

    /// push block to end of vector
    pub fn push(&self, block_info: FinalityBlockInfo) {
        let mut blocks = self.blocks.lock();
        blocks.push(block_info);
    }

    /// pop block from end of vector
    pub fn pop(&self) -> Option<FinalityBlockInfo> {
        let mut blocks = self.blocks.lock();
        blocks.pop()
    }

    /// remove block from arbitrary place of vector and return it
    pub fn remove(&self, i: usize) -> FinalityBlockInfo {
        let mut blocks = self.blocks.lock();
        blocks.remove(i)
    }

    /// Get count of temporary blocks
    pub fn len(&self) -> usize {
        let blocks = self.blocks.lock();
        blocks.len()
    }

    /// Sort block by sequence number
    pub fn sort(&self) {
        let mut blocks = self.blocks.lock();
        blocks.sort();
    }

    /// get minimum sequence number of blocks
    pub fn get_min_seq_no(&self) -> u32 {
        let mut blocks = self.blocks.lock();
        blocks.sort();
        if blocks.len() != 0 {
            blocks[0].block.block().read_info().unwrap().seq_no()
        } else {
            0xFFFFFFFF
        }
    }

    /// get pointer to vector of temporary blocks
    pub fn get_vec(&self) -> Arc<Mutex<Vec<FinalityBlockInfo>>> {
        self.blocks.clone()
    }

    /// check if block with sequence number exists in temporary blocks
    pub fn exists(&self, seq_no: u32) -> bool {
        let blocks = self.blocks.lock();
        blocks
            .iter()
            .find(|x| x.block.block().read_info().unwrap().seq_no() == seq_no)
            .is_some()
    }
}
