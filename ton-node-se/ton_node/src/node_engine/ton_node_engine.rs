use super::*;
use crate::error::NodeResult;
use crate::node_engine::stub_receiver::StubReceiver;
use crate::node_engine::DocumentsDb;
use crate::node_engine::documents_db_mock::DocumentsDbMock;
use parking_lot::Mutex;
use std::{
    cmp::Ordering,
    io::ErrorKind,
    sync::{
        atomic::Ordering as AtomicOrdering,
        atomic::AtomicU32,
        atomic::AtomicUsize,
        Arc,
    },
    time::{Duration, Instant},
};
use ton_api::ton::ton_engine::NetworkProtocol;
use ton_executor::BlockchainConfig;
use ton_types::HashmapType;

type PeerId = u64;
type TimerToken = usize;

#[cfg(test)]
#[path = "../../../tonos-se-tests/unit/test_ton_node_engine.rs"]
mod tests;

type Storage = FileBasedStorage;
type ArcBlockFinality = Arc<Mutex<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;
type ArcMsgProcessor = Arc<Mutex<MessagesProcessor<Storage>>>;
type ArcBlockApplier =
    Arc<Mutex<NewBlockApplier<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>>;

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
    receivers: Vec<Arc<Mutex<Box<dyn MessagesReceiver>>>>,
    live_control_receiver: Box<dyn LiveControlReceiver>,

    interval: usize,
    validators_by_shard: Arc<Mutex<HashMap<ShardIdent, Vec<i32>>>>,
    get_block_timout: u64,
    gen_block_timer: u64,
    reset_sync_limit: usize,

    pub msg_processor: ArcMsgProcessor,
    pub finalizer: ArcBlockFinality,
    pub block_applier: ArcBlockApplier,
    pub message_queue: Arc<InMessagesQueue>,
    pub incoming_blocks: IncomingBlocksCache,

    pub last_step: AtomicU32,

    // network handler part
    timers_count: AtomicUsize,
    timers: Arc<Mutex<HashMap<TimerToken, TimerHandler>>>,
    requests: Arc<Mutex<HashMap<PeerId, HashMap<u32, RequestCallback>>>>,

    responses: Arc<Mutex<Vec<(NetworkProtocol, ResponseCallback)>>>,
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

        let mut validators_by_shard = HashMap::new();
        validators_by_shard.insert(shard.clone(), vec![_node_index as i32]);
        let live_properties = Arc::new(EngineLiveProperties::new());

        message_queue.set_ready(true);

        let mut receivers: Vec<Arc<Mutex<Box<dyn MessagesReceiver>>>> = receivers
            .into_iter()
            .map(|r| Arc::new(Mutex::new(r)))
            .collect();

        //TODO: remove on production or use only for tests
        receivers.push(Arc::new(Mutex::new(Box::new(StubReceiver::with_params(
            shard.workchain_id() as i8,
            block_finality.lock().get_last_seq_no(),
            0,
        )))));

        Ok(TonNodeEngine {
            shard_ident: shard.clone(),
            receivers,
            live_properties,
            live_control_receiver,
            last_step: AtomicU32::new(100500),

            interval: 1,
            validators_by_shard: Arc::new(Mutex::new(validators_by_shard)),
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
            block_applier: Arc::new(Mutex::new(NewBlockApplier::with_params(
                block_finality.clone(),
                documents_db.clone(),
            ))),
            message_queue,
            incoming_blocks: IncomingBlocksCache::new(),
            timers_count: AtomicUsize::new(0),
            timers: Arc::new(Mutex::new(HashMap::new())),
            responses: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(HashMap::new())),
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

    //    pub fn validators(&self) -> usize {
    //        self.validators
    //    }

    pub fn validators(&self, shard: &ShardIdent) -> Vec<i32> {
        match self.validators_by_shard.lock().get(shard) {
            None => Vec::new(),
            Some(vals) => vals.to_vec(),
        }
    }

    pub fn validators_for_account(&self, wc: i32, id: &AccountId) -> Vec<i32> {
        for (shard, vals) in self.validators_by_shard.lock().iter() {
            if is_in_current_shard(shard, wc, id) {
                return vals.to_vec();
            }
        }
        Vec::new()
    }

    pub fn is_active_on_step(&self, step: usize, shard: &ShardIdent, val: i32) -> bool {
        let vals = self.validators(&shard);
        self.get_active_on_step(step, &vals) == val
    }

    pub fn push_message(&self, mut message: QueuedMessage, warning: &str, micros: u64) {
        while let Err(msg) = self.message_queue.queue(message) {
            message = msg;
            log::warn!(target: "node", "{}", warning);
            thread::sleep(Duration::from_micros(micros));
        }
    }

    pub fn get_active_on_step(&self, step: usize, vals: &Vec<i32>) -> i32 {
        /*
                let step = if (vals.len() % self.interval) == 0 {
                    step / self.interval
                } else {
                    step
                };
        */
        vals[step % vals.len()]
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

    /// Register new timer
    /// Only before start NetworkHandler
    #[allow(unused_assignments)]
    pub fn register_timer(&self, timeout: Duration, callback: TimerCallback) {
        let timers_count = self.timers_count.fetch_add(1, AtomicOrdering::SeqCst);
        let th = TimerHandler::with_params(timers_count, timeout, callback);
        self.timers.lock().insert(timers_count, th);
    }

    pub fn register_response_callback(
        &self,
        response: NetworkProtocol,
        callback: ResponseCallback,
    ) {
        self.responses.lock().push((response, callback));
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

///
/// Protocol packets
///
type PacketId = u8;
pub const REQUEST: PacketId = 1;
pub const RESPONSE: PacketId = 2;

impl TonNodeEngine {
    fn initialize(&self) {
    }

    // fn read(&self, peer: &PeerId, packet_id: u8, data: &[u8]) {
    //     let res = self.read_internal(peer, packet_id, data);
    //     if res.is_err() {
    //         warn!(target: "node", "Error in TonNodeEngine.read: {:?}", res);
    //     }
    // }

    fn timeout(&self, timer: TimerToken) {
        //debug!("TIMEOUT");
        let callback = {
            let timers = self.timers.lock();
            if let Some(handler) = timers.get(&timer) {
                Some(handler.get_callback())
            } else {
                None
            }
        };
        if let Some(callback) = callback {
            callback(self);
        }
        /*
                let timers = self.timers.lock();
                if timers.contains_key(&timer) {
                    if let Some(handler) = timers.get(&timer) {
                        let timer = handler.get_callback();
                        timer(self);
                    }
                }
        */
    }
}

type TimerCallback = fn(engine: &TonNodeEngine);
type RequestCallback = fn(
    engine: &TonNodeEngine,
    peer: &PeerId,
    reply: NetworkProtocol,
) -> NodeResult<()>;
type ResponseCallback = fn(
    engine: &TonNodeEngine,
    peer: &PeerId,
    reply: NetworkProtocol,
) -> NodeResult<NetworkProtocol>;

struct TimerHandler {
    // TODO: timer_id: TimerToken,
    timeout: Duration,
    callback: TimerCallback,
}

impl TimerHandler {
    pub fn with_params(_timer_id: TimerToken, timeout: Duration, callback: TimerCallback) -> Self {
        TimerHandler {
            // TODO: timer_id,
            timeout,
            callback,
        }
    }

    pub fn get_callback(&self) -> TimerCallback {
        let callback = self.callback;
        callback
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
