use super::*;
#[allow(deprecated)]
use crate::error::NodeResult;
use crate::ethcore_network_devp2p::NetworkService;
use parking_lot::Mutex;
use poa::engines::authority_round::subst::{Client, EngineClient, EthereumMachine, U256};
use poa::engines::authority_round::{AuthorityRound, AuthorityRoundParams};
use poa::engines::validator_set::{/*PublicKeyListImport,*/ SimpleList};
use poa::engines::Engine;
use std::cmp::Ordering;
use std::io::ErrorKind;
use std::sync::{atomic::Ordering as AtomicOrdering, atomic::{AtomicBool, AtomicUsize}, Arc};
use std::time::Duration;
use ton_api::ton::ton_engine::{NetworkProtocol, network_protocol::*};
use ton_api::{BoxedDeserialize, BoxedSerialize, IntoBoxed};
use ton_executor::BlockchainConfig;

#[cfg(test)]
#[path = "../../../tonos-se-tests/unit/test_ton_node_engine.rs"]
mod tests;

#[derive(Clone, Debug)]
pub enum BlockData {
    Block(Vec<u8>),
    EmptyStep(Vec<u8>),
    None,
}

type Storage = FileBasedStorage;
type ArcBlockFinality = Arc<Mutex<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;
type ArcMsgProcessor = Arc<Mutex<MessagesProcessor<Storage>>>;
type ArcBlockApplier = Arc<Mutex<NewBlockApplier<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>>;

// TODO do interval and validater field of TonNodeEngine
// and read from config
pub const INTERVAL: usize = 2;
pub const VALIDATORS: usize = 7;

pub const GEN_BLOCK_TIMEOUT: u64 = 400;//800;
pub const GEN_BLOCK_TIMER: u64 = 20;
pub const RESET_SYNC_LIMIT: usize = 5;

/// It is top level struct provided node functionality related to transactions processing.
/// Initialises instances of: all messages receivers, InMessagesQueue, MessagesProcessor.
pub struct TonNodeEngine {    
    local: bool,
    shard_ident: ShardIdent,

    receivers: Vec<Arc<Mutex<Box<dyn MessagesReceiver>>>>,

    interval: usize,
    validators_by_shard: Arc<Mutex<HashMap<ShardIdent, Vec<i32>>>>,
    get_block_timout: u64,
    gen_block_timer: u64,
    reset_sync_limit: usize,

    pub msg_processor: ArcMsgProcessor,
    pub finalizer: ArcBlockFinality,
    pub block_applier: ArcBlockApplier,
    pub message_queue: Arc<InMessagesQueue>,
    pub analyzers: Arc<Mutex<HashMap<u32, ConfirmationAnalizer>>>,
    pub poa_context: Arc<Mutex<PoAContext>>,
    pub incoming_blocks: IncomingBlocksCache,

    validator_index: u8,
    pub last_step: AtomicUsize,
    pub is_synchronize: AtomicBool,
    pub is_self_block_processing: AtomicBool,
    pub is_starting: AtomicBool,
    pub sync_limit: AtomicUsize,

    // network handler part
    peer_list: Arc<Mutex<Vec<PeerId>>>,
    timers_count: AtomicUsize,
    timers: Arc<Mutex<HashMap<TimerToken, TimerHandler>>>,
    requests: Arc<Mutex<HashMap<PeerId, HashMap<u32, RequestCallback>>>>,

    responses: Arc<Mutex<HashMap<NetworkProtocol, ResponseCallback>>>,
    is_network_enabled: bool,
    service: Arc<NetworkService>,
    #[cfg(test)]
    pub test_counter_in: Arc<Mutex<u32>>,
    #[cfg(test)]
    pub test_counter_out: Arc<Mutex<u32>>,

    routing_table: RoutingTable,

    pub db: Arc<Box<dyn DocumentsDb>>,
}

impl TonNodeEngine {

    pub fn start(self: Arc<Self>) -> NodeResult<()> {

        for recv in self.receivers.iter() {
            recv.lock().run(Arc::clone(&self.message_queue))?;
        }

        if self.is_network_enabled { 
            self.service.start().expect("Error starting service");
        }
        self.service
                .register_protocol(self.clone(), *b"tvm", &[(1u8, 20u8)])
                .unwrap();

        let node = self.clone();
        if node.local {
            node.is_starting.store(false, AtomicOrdering::SeqCst);
            thread::spawn(move || {
                let timestamp =
                    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as u32;
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
        } else {
            std::thread::spawn(move ||{ 
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    node.set_ready_mode_timer();
                }
            });
        }

        Ok(())

    }

    pub fn stop(self: Arc<Self>) -> NodeResult<()> {
        self.service.stop();
        info!(target: "node","TONNodeEngine stoped.");
        Ok(())
    }

    /// Construct new engine for selected shard
    /// with given time to generate block candidate
    pub fn with_params(
        shard: ShardIdent,
        local: bool,
        port: u16, 
        node_index: u8,
        _poa_validators: u16,
        poa_interval: u16,
        private_key: Keypair,
        public_keys: Vec<ed25519_dalek::PublicKey>,
        boot_list: Vec<String>,
        receivers: Vec<Box<dyn MessagesReceiver>>,
        blockchain_config: BlockchainConfig,
        documents_db: Option<Box<dyn DocumentsDb>>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {

        let mut config = NetworkConfiguration::new_with_port(port);
        info!(target: "node", "boot nodes:");
        for n in boot_list.iter() {
            info!(target: "node", "{}", n);
        }
        config.boot_nodes = boot_list;
        config.use_secret = get_devp2p_secret(&private_key);

        let documents_db = Arc::new(documents_db.unwrap_or_else(|| Box::new(DocumentsDbMock)));
        
        let queue = Arc::new(InMessagesQueue::with_db(shard.clone(), 10000, documents_db.clone()));

        let storage = Arc::new(Storage::with_path(shard.clone(), storage_path.clone())?);
        let block_finality = Arc::new(Mutex::new(
            OrdinaryBlockFinality::with_params(
                shard.clone(),
                storage_path,
                storage.clone(),
                storage.clone(),
                storage.clone(),
                storage.clone(),
                Some(documents_db.clone()),
                Vec::new(),
            )));
        let res = block_finality.lock().load();
        if res.is_ok() {
            info!(target: "node", "load block finality successuly");
        } else {
            match res.unwrap_err() {
                NodeError(NodeErrorKind::Io(err), _) => {
                    if err.kind() != ErrorKind::NotFound {
                        bail!(err);
                    }
                },
                err => {
                    bail!(err);
                }
            }
        }

        let network_service = Arc::new(NetworkService::new(config, None)?);
        let mut validators_by_shard = HashMap::new();
        validators_by_shard.insert(shard.clone(), vec![node_index as i32]);
        block_finality.lock().finality.add_signer(public_keys[node_index as usize].clone());
        let poa = Arc::new(Mutex::new(TonNodeEngine::create_poa(private_key, public_keys, poa_interval)));
        poa.lock().engine.set_validator(node_index as usize);
   
        if local {
            queue.set_ready(true);
        }

        let mut receivers: Vec<Arc<Mutex<Box<dyn MessagesReceiver>>>> = 
            receivers.into_iter().map(|r| Arc::new(Mutex::new(r))).collect();

        //TODO: remove on production or use only for tests
        receivers.push(Arc::new(Mutex::new(Box::new(StubReceiver::with_params(
            shard.workchain_id() as i8, block_finality.lock().get_last_seq_no(), 0)))));

        Ok(TonNodeEngine {
            local,
            shard_ident: shard.clone(),
            receivers,
            validator_index: node_index, 
            last_step: AtomicUsize::new(100500),
            is_synchronize: AtomicBool::new(false),
            is_self_block_processing: AtomicBool::new(false),
            is_starting: AtomicBool::new(false),
            sync_limit: AtomicUsize::new(0),

            interval: poa_interval as usize,
            validators_by_shard: Arc::new(Mutex::new(validators_by_shard)),
            get_block_timout: GEN_BLOCK_TIMEOUT,
            gen_block_timer: GEN_BLOCK_TIMER,
            reset_sync_limit: RESET_SYNC_LIMIT,

            msg_processor: Arc::new(Mutex::new(
                MessagesProcessor::with_params(
                    queue.clone(), 
                    storage.clone(), 
                    shard.clone(),
                    blockchain_config,
                )
            )),
            finalizer: block_finality.clone(),
            block_applier: Arc::new(Mutex::new(NewBlockApplier::with_params(block_finality.clone(), documents_db.clone()))),
            message_queue: queue.clone(),
            analyzers: Arc::new(Mutex::new(HashMap::new())),
            poa_context: poa,
            incoming_blocks: IncomingBlocksCache::new(),
            is_network_enabled: port != 0,
            service: network_service,
            peer_list: Arc::new(Mutex::new(vec![])),
            timers_count: AtomicUsize::new(0),
            timers: Arc::new(Mutex::new(HashMap::new())),
            responses: Arc::new(Mutex::new(HashMap::new())),
            requests: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(test)]
            test_counter_out: Arc::new(Mutex::new(0)),
            #[cfg(test)]
            test_counter_in: Arc::new(Mutex::new(0)),

            db: documents_db,
            routing_table: RoutingTable::new(shard)
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
            Some(vals) => vals.to_vec()
        }
    }

    pub fn validators_for_account(&self, wc: i32, id: &AccountId) -> Vec<i32> {
        for (shard, vals) in self.validators_by_shard.lock().iter() {
            if messages::is_in_current_shard(shard, wc, id) {
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
            std::thread::sleep(std::time::Duration::from_micros(micros));
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

    pub fn set_validator(&self, shard: &ShardIdent, val: i32) {
        self.remove_validator(val);
        if shard == self.current_shard_id() {
            let engine = &self.poa_context.lock().engine;
            let val = val as usize;
            engine.set_validator(val);
            self.finalizer.lock().finality.add_signer(engine.validator_key(val));
        }
        let mut vals = self.validators_by_shard.lock();
        let vals = vals.entry(shard.clone()).or_insert(Vec::new());
        for i in 0..vals.len() {
            if vals[i] > val {
                vals.insert(i, val);
            }
            if vals[i] == val {
                return;           
            }
        }
        vals.push(val);
    }

    pub fn remove_validator(&self, val: i32) -> bool {
        for (shard, vals) in self.validators_by_shard.lock().iter_mut() {
            for i in 0..vals.len() {
                if vals[i] == val {
                    if shard == self.current_shard_id() {
                        let engine = &self.poa_context.lock().engine;
                        let val = val as usize;
                        self.finalizer.lock().finality.remove_signer(&engine.validator_key_id(val));
                        engine.remove_validator(val);
                    }
                    vals.remove(i);
                    return true;
                }
            }
        }
        false
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

    /// Getter for local node mode
    pub fn is_local(&self) -> bool {
        self.local
    }

    /// Getter for validator index of node
    pub fn validator_index(&self) -> i32 {
        self.validator_index as i32
    }

    /// Getter for routing table
    pub fn routing_table(&self) -> &RoutingTable {
        &self.routing_table
    }

    ///
    /// Get network service
    /// 
    pub fn get_network_service(&self) -> Arc<NetworkService> {
        self.service.clone()
    }

    ///
    /// Get devp2p external url of current server
    /// 
    pub fn external_url(&self) -> Option<String>{
        self.service.external_url()
    }

    fn read_internal(&self, io: &dyn NetworkContext, peer: &PeerId, packet_id: u8, data: &[u8]) -> NodeResult<()> {
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
                
                let funcs = self.responses.lock().iter()
                    .filter(|(key, _val)| variant_eq(key.clone(), &request))
                    .map(|(_key, val)| val.clone()).collect::<Vec<ResponseCallback>>();

                if funcs.len() == 1 {
                    let responce = funcs[0](self, io, peer, request);
                    if responce.is_err() {
                        //send error response
                        let error = ton_api::ton::ton_engine::network_protocol::networkprotocol::Error{
                            err_code: -1,
                            msg: format!("{:?}", responce.unwrap_err()).to_string(),
                        };
                        self.send_response(io, peer, request_id, &error.into_boxed())?;
                    } else {
                        // send som respnse
                        self.send_response(io, peer, request_id, &responce.unwrap())?;
                    }
                }
            },
            RESPONSE => {
                // reply processing
                let mut r = [0u8; 4];
                r.copy_from_slice(&data[0..4]);
                let request_id = u32::from_be_bytes(r);
                
                let callback = self.requests.lock().get_mut(peer).unwrap().remove(&request_id);
                if let Some(callback) = callback {
                    let request = deserialize::<NetworkProtocol>(&mut &data[4..]);
                    callback(self, io, peer, request)?;
                }
            },
            x => {
                warn!(target: "node", "Unknown packet id: {}", x);
            }
        }

        Ok(())
    }

    ///
    /// Send request to peer
    /// 
    pub fn send_request(&self,
        io: &dyn NetworkContext, 
        peer: &PeerId, 
        request: &NetworkProtocol, 
        callback: RequestCallback
    ) -> NodeResult<()> {
        
        let mut r_data = serialize(request)?;
        let request_id = rand::random::<u32>();
        let mut data = request_id.to_be_bytes().to_vec();
        data.append(&mut r_data);
        io.send(*peer, REQUEST, data.clone())?;
        self.requests.lock()
            .entry(peer.clone()).or_insert(HashMap::new())
            .insert(request_id, callback);
        Ok(())
    }

    fn send_response(&self,
        io: &dyn NetworkContext, 
        peer: &PeerId, 
        request_id: u32,
        response: &NetworkProtocol
    ) -> NodeResult<()> {
        
        let mut r_data = serialize(response)?;
        let mut data = request_id.to_be_bytes().to_vec();
        data.append(&mut r_data);

        io.send(*peer, RESPONSE, data)?;
        Ok(())
    }

    /// Register new timer
    /// Only before start NetworkHandler
    #[allow(unused_assignments)]
    pub fn register_timer(&self,
        timeout: Duration,
        callback: TimerCallback)
    {
        let timers_count = self.timers_count.fetch_add(1, AtomicOrdering::SeqCst);
        let th = TimerHandler::with_params(
            timers_count,
            timeout, 
            callback, 
        );
        self.timers.lock().insert(
            timers_count, 
            th,
        );
    }

    pub fn register_response_callback(&self,
        responce: NetworkProtocol,
        callback: ResponseCallback)
    {
        self.responses.lock().insert(responce, callback);
    }

    fn disconnected_internal(&self, io: &dyn NetworkContext, peer: &PeerId ) -> NodeResult<()> {
        let mut peer_list = self.peer_list.lock();
        let index = peer_list.iter().position(|x| *x == peer.clone());
        if let Some(index) = index {
            peer_list.remove(index);
            info!(target: "node", "client {} deleted from peer list", peer);

            if let Some(r) = self.requests.lock().remove(peer) {
                for (_reuest_id, callback) in r.iter() {
                    let error = networkprotocol::Error{
                        err_code: -2,
                        msg: "peer disconnected unexpectedly".to_string(),
                    };
                    callback(self, io, peer, error.into_boxed())?;
                }
            }
        }
        Ok(())
    }

    fn create_poa(private_key: Keypair, public_keys: Vec<ed25519_dalek::PublicKey>, interval: u16) -> PoAContext {

        let params = AuthorityRoundParams {
            block_reward: U256::from(0u64),
            block_reward_contract: None,
            block_reward_contract_transition: 0,
            empty_steps_transition: 0,
            immediate_transitions: false,
            maximum_empty_steps: std::usize::MAX,
            maximum_uncle_count: 0,
            maximum_uncle_count_transition: 0,
            start_step: None,
            step_duration: interval,
            validate_score_transition: 0,
            validate_step_transition: 0,
            validators: Box::new(SimpleList::new(public_keys.clone())),
            ton_key: private_key,
        };
        let engine = AuthorityRound::new(params, EthereumMachine::new()).unwrap();
        let client: Arc<dyn EngineClient> = Arc::new(Client::new(engine.clone()));

        engine.register_client(Arc::downgrade(&client));
        PoAContext {
            engine,
            client,
        }
    }

    fn request_to_node_info(&self, io: &dyn NetworkContext, peer: &PeerId) -> NodeResult<()> {
        let request = networkprotocol::RequestNodeInfo {
            id: 0,
        };
        self.send_request(io, peer, &request.into_boxed(), TonNodeEngine::process_node_info)
    }
        
    fn process_node_info(&self, _io: &dyn NetworkContext, peer: &PeerId, request: NetworkProtocol) -> NodeResult<()> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_ResponseNodeInfo(info) => {
                
                let shard = ShardIdent::with_prefix_len(info.shard_pfx_len as u8, info.workchain, info.shard_prefix as u64)?;

                self.set_validator(&shard, info.validator_no);
                
                let node_info = NodeInfo::new(info.validator_no as usize, shard);                
                println!("ROUTING: insert peer {}, info {:?}", peer, node_info);
                self.routing_table().insert(node_info, peer.clone() as usize);
            } 
            NetworkProtocol::TonEngine_NetworkProtocol_Error(error) => {
                warn!(target: "node", "Get route node info FAILED: {}", error.msg);
            }
            _ => return node_err!(NodeErrorKind::TlIncompatiblePacketType)
        }
        Ok(())
    }

}

///
/// internal context
/// 
pub struct PoAContext {
    pub engine: Arc<AuthorityRound>,
    pub client: Arc<dyn EngineClient>,
}

///
/// Protocol packets
/// 
pub const REQUEST: PacketId = 1;
pub const RESPONSE: PacketId = 2;

impl NetworkProtocolHandler for TonNodeEngine{
    
    fn connected(&self, io: &dyn NetworkContext, peer: &PeerId) {
        let mut peer_list = self.peer_list.lock();
        peer_list.push(peer.clone());
        // send request to NodeInfo
        let res = self.request_to_node_info(io, peer);
        if res.is_err() {
            warn!(target: "node", "Request to node info failed: {:?}", res.unwrap_err());
        }
        info!(target: "node", "Client {} added to peer list", peer);
    }

    fn disconnected(&self, io: &dyn NetworkContext, peer: &PeerId) {
        let res = self.disconnected_internal(io, peer);
        // remove peer from routing table
        self.routing_table().remove_by_peer(peer);
        info!(target: "node", "Disconnected {}", peer);
        if res.is_err() {
            warn!(target: "node", "Error in disconnected: {:?}", res);
        }
    }
    
    fn initialize(&self, io: &dyn NetworkContext) {
        let timers = self.timers.lock();
        for (t_id, timer) in timers.iter() {
            let res = io.register_timer(t_id.clone(), timer.timeout);
            if res.is_err() {
                log::error!(target: "node", "Register timer error {:?}", res.unwrap_err());
            }
        }
    }
    
    fn read(&self, io: &dyn NetworkContext, peer: &PeerId, packet_id: u8, data: &[u8]) {
        let res = self.read_internal(io, peer, packet_id, data);
        if res.is_err() {
            warn!(target: "node", "Error in TonNodeEngine.read: {:?}", res);
        }
    }

    fn timeout(&self, io: &dyn NetworkContext, timer: TimerToken) {
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
            callback(self, io);
        }
/*
        let timers = self.timers.lock();
        if timers.contains_key(&timer) {
            if let Some(handler) = timers.get(&timer) {
                let timer = handler.get_callback();
                timer(self, io);
            }
        }
*/
    }

}

type TimerCallback = 
    fn(engine: &TonNodeEngine, io: &dyn NetworkContext);
type RequestCallback = 
    fn(engine: &TonNodeEngine, io: &dyn NetworkContext, peer: &PeerId, reply: NetworkProtocol) -> NodeResult<()>;
type ResponseCallback = 
    fn(engine: &TonNodeEngine, io: &dyn NetworkContext, peer: &PeerId, reply: NetworkProtocol) -> NodeResult<NetworkProtocol>;

#[derive(Clone)]
struct TimerHandler {
    timer_id: TimerToken,
    timeout: Duration,
    callback: TimerCallback,
}

impl TimerHandler {
    pub fn with_params(
        timer_id: TimerToken,
        timeout: Duration,
        callback: TimerCallback ) -> Self {
        TimerHandler { timer_id, timeout, callback }
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
            return node_err!(NodeErrorKind::TlSerializeError);
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
        },
    }
}

extern crate ethkey;

pub fn get_devp2p_secret(key: &Keypair) -> Option<ethkey::Secret> {
    let key = key.secret.to_bytes();
    ethkey::Secret::from_slice(&key)
}

pub struct ConfirmationContex {
    pub peer: PeerId,
    pub result: u8,
    pub block_seq_no: u32,
    pub block_start_lt: u64,
    pub block_end_lt: u64,
    pub block_gen_utime: u32,
}

impl ConfirmationContex {
    pub fn new() -> Self {
        Self {
            peer: 0,
            result: 0,
            block_seq_no: 0,
            block_start_lt: 0,
            block_end_lt: 0,
            block_gen_utime: 0,
        }
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut data = vec![];
        data.extend_from_slice(&(self.peer as u64).to_le_bytes());
        data.extend_from_slice(&(self.result).to_le_bytes());
        data.extend_from_slice(&(self.block_seq_no).to_le_bytes());
        data.extend_from_slice(&(self.block_start_lt).to_le_bytes());
        data.extend_from_slice(&(self.block_end_lt).to_le_bytes());
        data.extend_from_slice(&(self.block_gen_utime).to_le_bytes());
        data
    }

    pub fn deserialize(data: &mut impl Read) -> NodeResult<Self> {
        let mut res = Self::new();
        res.peer = data.read_le_u64()? as usize;
        res.result = data.read_byte()?;
        res.block_seq_no = data.read_le_u32()?;
        res.block_start_lt = data.read_le_u64()?;
        res.block_end_lt = data.read_le_u64()?;
        res.block_gen_utime = data.read_le_u32()?;
        Ok(res)
    }
}

///
/// Analyzer for confirmations
///
pub struct ConfirmationAnalizer {
    validators_count: i32,
    pub confirmations: Vec<networkprotocol::ConfirmValidation>,
    pub block_candidate: Block,
    pub applied: bool,
}

impl ConfirmationAnalizer {
    pub fn new(validators: i32) -> Self {
        Self {
            validators_count: validators,
            confirmations: vec![],
            block_candidate: Block::default(),
            applied: false,
        }
    }

    pub fn select_actual_validators(&self) -> Option<Vec<PeerId>> {
        let mut succedded_count = 0;
        let mut succedded_validators = vec![];

        for c in self.confirmations.iter() {
            if Self::is_success_status(c.result) {
                succedded_count += 1;
                succedded_validators.push(c.peer as PeerId);
            }
        }
        if succedded_count > (self.validators_count / 2) {
            Some(succedded_validators)
        } else {
            None
        }
    }

    pub fn is_success_status(result: i64) -> bool {
        result == 0 || result == 2
    }

    pub fn select_failed_validators(&self) -> Option<Vec<PeerId>> {
        let mut count = 0;
        let mut failed_validators = vec![];

        for c in self.confirmations.iter() {
            if !Self::is_success_status(c.result) {
                count += 1;
                failed_validators.push(c.peer as PeerId);
            }
        }
        if count > (self.validators_count / 2) {
            Some(failed_validators)
        } else {
            None
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
            .read_info().unwrap().seq_no()
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
        self.block.block().read_info().unwrap().seq_no() == other.block.block().read_info().unwrap().seq_no()
    }
}

pub struct IncomingBlocksCache {
    blocks: Arc<Mutex<Vec<FinalityBlockInfo>>>,
}

impl IncomingBlocksCache {
    /// create new instance of TempoparyBlocks
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

    /// get minimun sequence number of blocks
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