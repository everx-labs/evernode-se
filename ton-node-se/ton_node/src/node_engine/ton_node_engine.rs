use super::*;
use crate::error::NodeResult;
use crate::node_engine::stub_receiver::StubReceiver;
use crate::node_engine::DocumentsDb;
use crate::node_engine::documents_db_mock::DocumentsDbMock;
use parking_lot::Mutex;
use std::{
    io::ErrorKind,
    sync::{
        atomic::{Ordering, AtomicU32},
        Arc,
    },
    time::{Duration, Instant},
};
use ton_executor::BlockchainConfig;
use ton_types::HashmapType;

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
        self.time_delta.load(Ordering::Relaxed)
    }

    fn set_time_delta(&self, value: u32) {
        self.time_delta.store(value, Ordering::Relaxed)
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
    live_control_receiver: Option<Box<dyn LiveControlReceiver>>,

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
        if let Some(ref control_receiver) = self.live_control_receiver {
            control_receiver.run(Box::new(live_control))?;
        }


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
                        log::trace!(target: "node", "block generated successfully");
                    }
                    Ok(None) => {
                        log::trace!(target: "node", "block was not generated successfully");
                    }
                    Err(err) => {
                        log::warn!(target: "node", "failed block generation: {}", err);
                    }
                }
            }
        });

        Ok(())
    }

    pub fn stop(self: Arc<Self>) -> NodeResult<()> {
        log::info!(target: "node","TONNodeEngine stopped.");
        Ok(())
    }

    /// Construct new engine for selected shard
    /// with given time to generate block candidate
    pub fn with_params(
        shard: ShardIdent,
        _local: bool,
        _port: u16,
        _node_index: u8,
        private_key: Keypair,
        _public_keys: Vec<ed25519_dalek::PublicKey>,
        _boot_list: Vec<String>,
        receivers: Vec<Box<dyn MessagesReceiver>>,
        live_control_receiver: Option<Box<dyn LiveControlReceiver>>,
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
                log::info!(target: "node", "load block finality successfully");
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
            log::warn!(target: "node", "{}", warning);
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

    fn print_block_info(block: &Block) {
        let extra = block.read_extra().unwrap();
        log::info!(target: "node",
            "block: gen time = {}, in msg count = {}, out msg count = {}, account_blocks = {}",
            block.read_info().unwrap().gen_utime(),
            extra.read_in_msg_descr().unwrap().len().unwrap(),
            extra.read_out_msg_descr().unwrap().len().unwrap(),
            extra.read_account_blocks().unwrap().len().unwrap());
    }

    ///
    /// Generate new block if possible
    ///
    pub fn prepare_block(&self, timestamp: u32) -> NodeResult<Option<SignedBlock>> {

        let mut time = [0u128; 10];
        let mut now = Instant::now();

        log::debug!("PREP_BLK_START");
        let shard_state = self.finalizer.lock().get_last_shard_state();
        let blk_prev_info = self.finalizer.lock().get_last_block_info()?;
        log::info!(target: "node", "PARENT block: {:?}", blk_prev_info);
        let seq_no = self.finalizer.lock().get_last_seq_no() + 1;
        let gen_block_time = Duration::from_millis(self.gen_block_timeout());
        time[0] = now.elapsed().as_micros();
        now = Instant::now();

        let result = self.msg_processor.lock().generate_block_multi(
            &shard_state,
            gen_block_time,
            seq_no,
            blk_prev_info,
            timestamp,
            true, //tvm code tracing enabled by default on trace level
        );
        let (block, new_shard_state) = match result? {
            Some(result) => result,
            None => return Ok(None)
        };
        time[1] = now.elapsed().as_micros();
        now = Instant::now();
        time[2] = now.elapsed().as_micros();
        now = Instant::now();
        time[3] = now.elapsed().as_micros();
        now = Instant::now();

        log::debug!("PREP_BLK_AFTER_GEN");
        log::debug!("PREP_BLK2");

        //        self.message_queue.set_ready(false);

        // TODO remove debug print
        Self::print_block_info(&block);

        /*let res = self.propose_block_to_db(&mut block);

        if res.is_err() {
            log::warn!(target: "node", "Error propose_block_to_db: {}", res.unwrap_err());
        }*/

        time[4] = now.elapsed().as_micros();
        now = Instant::now();

        time[5] = now.elapsed().as_micros();
        now = Instant::now();

        let s_block = SignedBlock::with_block_and_key(block, &self.private_key)?;
        let mut s_block_data = Vec::new();
        s_block.write_to(&mut s_block_data)?;
        time[6] = now.elapsed().as_micros();
        now = Instant::now();
        let (_, details) = self.finality_and_apply_block(
            &s_block,
            &s_block_data,
            new_shard_state,
            false,
        )?;

        time[7] = now.elapsed().as_micros();
        log::info!(target: "profiler",
            "{} {} / {} / {} / {} / {} / {} micros",
            "Prepare block time: setup/gen/analysis1/analysis2/seal/finality",
            time[0], time[1], time[2], time[3], time[4] + time[5], time[6] + time[7]
        );

        let mut str_details = String::new();
        for detail in details {
            str_details = if str_details.is_empty() {
                format!("{}", detail)
            } else {
                format!("{} / {}", str_details, detail)
            }
        }
        log::info!(target: "profiler", "Block finality details: {} / {} micros",  time[6], str_details);

        log::debug!("PREP_BLK_SELF_STOP");

        Ok(Some(s_block))
    }

    /// finality and apply block
    fn finality_and_apply_block(
        &self,
        block: &SignedBlock,
        block_data: &[u8],
        applied_shard: ShardStateUnsplit,
        is_sync: bool,
    ) -> NodeResult<(Arc<ShardStateUnsplit>, Vec<u128>)> {

        let mut time = Vec::new();
        let now = Instant::now();
        let hash = block.block().hash().unwrap();
        let finality_hash = vec!(hash);
        let new_state = self.block_applier.lock().apply(
            block,
            Some(block_data.to_vec()),
            finality_hash,
            applied_shard,
            is_sync,
        )?;
        time.push(now.elapsed().as_micros());
        Ok((new_state, time))
    }
}

pub fn get_config_params(json: &str) -> (NodeConfig, Vec<ed25519_dalek::PublicKey>) {
    match NodeConfig::parse(json) {
        Ok(config) => match config.import_keys() {
            Ok(keys) => (config, keys),
            Err(err) => {
                log::warn!(target: "node", "{}", err);
                panic!("{} / {}", err, json)
            }
        },
        Err(err) => {
            log::warn!(target: "node", "Error parsing configuration file. {}", err);
            panic!("Error parsing configuration file. {}", err)
        }
    }
}

#[derive(Clone, Debug)]
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
        self.blocks.lock().push(block_info);
    }

    /// pop block from end of vector
    pub fn pop(&self) -> Option<FinalityBlockInfo> {
        self.blocks.lock().pop()
    }

    /// remove block from arbitrary place of vector and return it
    pub fn remove(&self, i: usize) -> FinalityBlockInfo {
        self.blocks.lock().remove(i)
    }

    /// Get count of temporary blocks
    pub fn len(&self) -> usize {
        self.blocks.lock().len()
    }

    /// get pointer to vector of temporary blocks
    pub fn get_vec(&self) -> Arc<Mutex<Vec<FinalityBlockInfo>>> {
        self.blocks.clone()
    }

    /// check if block with sequence number exists in temporary blocks
    pub fn exists(&self, seq_no: u32) -> bool {
        self.blocks.lock()
            .iter()
            .find(|x| x.block.block().read_info().unwrap().seq_no() == seq_no)
            .is_some()
    }
}
