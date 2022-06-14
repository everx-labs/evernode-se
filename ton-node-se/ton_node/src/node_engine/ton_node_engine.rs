use super::*;
#[allow(deprecated)]
use crate::error::NodeResult;
use crate::node_engine::stub_receiver::StubReceiver;
use crate::node_engine::DocumentsDb;
use node_engine::documents_db_mock::DocumentsDbMock;
use parking_lot::Mutex;
use std::cmp::Ordering;
use std::io::ErrorKind;
use std::sync::atomic::AtomicU32;
use std::sync::{
    atomic::Ordering as AtomicOrdering,
    atomic::AtomicUsize,
    Arc,
};
use std::time::Duration;
use ton_api::ton::ton_engine::{network_protocol::*, NetworkProtocol};
use ton_api::{BoxedDeserialize, BoxedSerialize, IntoBoxed};
use ton_executor::BlockchainConfig;

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

    validator_index: u8,
    pub last_step: AtomicUsize, // AtomicU32

    // network handler part
    peer_list: Arc<Mutex<Vec<PeerId>>>,
    timers_count: AtomicUsize,
    timers: Arc<Mutex<HashMap<TimerToken, TimerHandler>>>,
    requests: Arc<Mutex<HashMap<PeerId, HashMap<u32, RequestCallback>>>>,

    responses: Arc<Mutex<Vec<(NetworkProtocol, ResponseCallback)>>>,
    is_network_enabled: bool,
    #[cfg(test)]
    pub test_counter_in: Arc<Mutex<u32>>,
    #[cfg(test)]
    pub test_counter_out: Arc<Mutex<u32>>,

    pub(crate) private_key: Keypair,

    pub db: Arc<Box<dyn DocumentsDb>>,
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
                let res = node.prepare_block(timestamp);
                if res.is_ok() {
                    trace!(target: "node", "block generated successfully");
                } else {
                    warn!(target: "node", "failed block generation: {:?}", res.unwrap_err());
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
        port: u16,
        node_index: u8,
        private_key: Keypair,
        public_keys: Vec<ed25519_dalek::PublicKey>,
        boot_list: Vec<String>,
        receivers: Vec<Box<dyn MessagesReceiver>>,
        live_control_receiver: Box<dyn LiveControlReceiver>,
        blockchain_config: BlockchainConfig,
        documents_db: Option<Box<dyn DocumentsDb>>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        info!(target: "node", "boot nodes:");
        for n in boot_list.iter() {
            info!(target: "node", "{}", n);
        }

        let documents_db = Arc::new(documents_db.unwrap_or_else(|| Box::new(DocumentsDbMock)));

        let queue = Arc::new(InMessagesQueue::with_db(
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

        let mut validators_by_shard = HashMap::new();
        validators_by_shard.insert(shard.clone(), vec![node_index as i32]);
        let live_properties = Arc::new(EngineLiveProperties::new());
        // private_key,
        // public_keys,
        // poa_interval,
        // live_properties.time_delta.clone(),

        queue.set_ready(true);

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
            validator_index: node_index,
            last_step: AtomicUsize::new(100500),

            interval: 1,
            validators_by_shard: Arc::new(Mutex::new(validators_by_shard)),
            get_block_timout: GEN_BLOCK_TIMEOUT,
            gen_block_timer: GEN_BLOCK_TIMER,
            reset_sync_limit: RESET_SYNC_LIMIT,

            msg_processor: Arc::new(Mutex::new(MessagesProcessor::with_params(
                queue.clone(),
                storage.clone(),
                shard.clone(),
                blockchain_config,
            ))),
            finalizer: block_finality.clone(),
            block_applier: Arc::new(Mutex::new(NewBlockApplier::with_params(
                block_finality.clone(),
                documents_db.clone(),
            ))),
            message_queue: queue.clone(),
            incoming_blocks: IncomingBlocksCache::new(),
            is_network_enabled: port != 0,
            peer_list: Arc::new(Mutex::new(vec![])),
            timers_count: AtomicUsize::new(0),
            timers: Arc::new(Mutex::new(HashMap::new())),
            responses: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(test)]
            test_counter_out: Arc::new(Mutex::new(0)),
            #[cfg(test)]
            test_counter_in: Arc::new(Mutex::new(0)),

            db: documents_db,
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
            warn!(target: "node", "{}", warning);
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

    /// Getter for validator index of node
    pub fn validator_index(&self) -> i32 {
        self.validator_index as i32
    }

    fn read_internal(
        &self,
        peer: &PeerId,
        packet_id: u8,
        data: &[u8],
    ) -> NodeResult<()> {
        #[cfg(test)]
        debug!(target: "node",
            "\nReceived {} ({} bytes) from {}",
            packet_id,
            data.len(),
            peer
        );
        match packet_id {
            REQUEST => {
                // request processing
                let mut r = [0u8; 4];
                r.copy_from_slice(&data[0..4]);
                let request_id = u32::from_be_bytes(r);
                let request = deserialize::<NetworkProtocol>(&mut &data[4..]);

                let funcs = self
                    .responses
                    .lock()
                    .iter()
                    .filter(|(key, _val)| variant_eq(key, &request))
                    .map(|(_key, val)| val.clone())
                    .collect::<Vec<ResponseCallback>>();

                if funcs.len() == 1 {
                    let response = funcs[0](self, peer, request);
                    // if response.is_err() {
                    //     //send error response
                    //     let error = networkprotocol::Error {
                    //         err_code: -1,
                    //         msg: format!("{:?}", response.unwrap_err()).to_string(),
                    //     };
                    //     self.send_response(peer, request_id, &error.into_boxed())?;
                    // } else {
                    //     // send som response
                    //     self.send_response(peer, request_id, &response.unwrap())?;
                    // }
                }
            }
            RESPONSE => {
                // reply processing
                let mut r = [0u8; 4];
                r.copy_from_slice(&data[0..4]);
                let request_id = u32::from_be_bytes(r);

                let callback = self
                    .requests
                    .lock()
                    .get_mut(peer)
                    .unwrap()
                    .remove(&request_id);
                if let Some(callback) = callback {
                    let request = deserialize::<NetworkProtocol>(&mut &data[4..]);
                    callback(self, peer, request)?;
                }
            }
            x => {
                warn!(target: "node", "Unknown packet id: {}", x);
            }
        }

        Ok(())
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

    fn disconnected_internal(&self, peer: &PeerId) -> NodeResult<()> {
        let mut peer_list = self.peer_list.lock();
        let index = peer_list.iter().position(|x| *x == peer.clone());
        if let Some(index) = index {
            peer_list.remove(index);
            info!(target: "node", "client {} deleted from peer list", peer);

            if let Some(r) = self.requests.lock().remove(peer) {
                for (_request_id, callback) in r.iter() {
                    let error = networkprotocol::Error {
                        err_code: -2,
                        msg: "peer disconnected unexpectedly".to_string(),
                    };
                    callback(self, peer, error.into_boxed())?;
                }
            }
        }
        Ok(())
    }

    // fn create_poa(
    //     private_key: Keypair,
    //     public_keys: Vec<ed25519_dalek::PublicKey>,
    //     interval: u16,
    //     delta: Arc<AtomicU32>,
    // ) -> PoAContext {
    //     let params = AuthorityRoundParams {
    //         delta,
    //         block_reward: U256::from(0u64),
    //         block_reward_contract: None,
    //         block_reward_contract_transition: 0,
    //         empty_steps_transition: 0,
    //         immediate_transitions: false,
    //         maximum_empty_steps: std::usize::MAX,
    //         maximum_uncle_count: 0,
    //         maximum_uncle_count_transition: 0,
    //         start_step: None,
    //         step_duration: interval,
    //         validate_score_transition: 0,
    //         validate_step_transition: 0,
    //         validators: Box::new(SimpleList::new(public_keys.clone())),
    //         ton_key: private_key,
    //     };
    //     let engine = AuthorityRound::new(params, EthereumMachine::new()).unwrap();
    //     let client: Arc<dyn EngineClient> = Arc::new(Client::new(engine.clone()));

    //     engine.register_client(Arc::downgrade(&client));
    //     PoAContext { engine, client }
    // }

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

    fn read(&self, peer: &PeerId, packet_id: u8, data: &[u8]) {
        let res = self.read_internal(peer, packet_id, data);
        if res.is_err() {
            warn!(target: "node", "Error in TonNodeEngine.read: {:?}", res);
        }
    }

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

pub fn serialize<T: BoxedSerialize>(object: &T) -> NodeResult<Vec<u8>> {
    let mut ret = Vec::<u8>::new();
    {
        let mut serializer = ton_api::Serializer::new(&mut ret);
        let err = serializer.write_boxed(object);
        if err.is_err() {
            return Err(NodeError::TlSerializeError);
        }
    }
    Ok(ret)
}

pub fn deserialize<T: BoxedDeserialize>(bytes: &mut &[u8]) -> T {
    ton_api::Deserializer::new(bytes).read_boxed().unwrap()
}

fn variant_eq<T>(a: &T, b: &T) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
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
