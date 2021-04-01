use crate::error::NodeError;
use super::*;
use poa::engines::{Engine, Seal, authority_round::subst::{
    ExecutedBlock, OrdinaryHeader, TONBlock, UnsignedBlock, H256
}};
use std::{time::Instant, sync::atomic::Ordering as AtomicOrdering, io::Cursor};
use ton_api::{IntoBoxed, ton::ton_engine::{network_protocol::*, NetworkProtocol}};
use ton_block::{Deserializable, UnixTime32};
use ton_types::{serialize_toc, HashmapType};

pub fn init_ton_node_handlers(ton: &TonNodeEngine) {
    // register timers
    ton.register_timer(Duration::from_millis(ton.gen_block_timer()), TonNodeEngine::gen_timer );
    ton.register_timer(Duration::from_millis(ton.gen_block_timeout()), TonNodeEngine::reset_sync_timer );
    ton.register_timer(Duration::from_millis(ton.gen_block_timer()), TonNodeEngine::route_message_timer );
    //ton.register_timer(Duration::from_millis(GEN_BLOCK_TIMER), TonNodeEngine::set_ready_mode_timer );

    // register supported requests
    let request = networkprotocol::ValidationRequest::default();
    ton.register_response_callback(request.into_boxed(), TonNodeEngine::verify_block);    

    let request = networkprotocol::EmptyStepRequest::default();
    ton.register_response_callback(request.into_boxed(), TonNodeEngine::verify_empty_step);  

    let request = networkprotocol::RequestBlockByNumber::default();
    ton.register_response_callback(request.into_boxed(), TonNodeEngine::request_block);  

    let request = networkprotocol::RequestNodeInfo::default();
    ton.register_response_callback(request.into_boxed(), TonNodeEngine::process_request_node_info);  

    let request = networkprotocol::SendMessageRequest::default();
    ton.register_response_callback(request.into_boxed(), TonNodeEngine::process_request_send_message);

    let request = networkprotocol::ReflectToDbRequest::default();
    ton.register_response_callback(request.into_boxed(), TonNodeEngine::process_request_reflect_to_db);
}

impl TonNodeEngine {

/*
    fn get_next_validator_index(&self, step_count: usize) -> usize {
        let next_step = self.last_step.load(AtomicOrdering::SeqCst) + (self.interval() as usize * step_count);
        next_step % self.validators()
    }
*/

    fn route_message_timer(&self, io: &dyn NetworkContext) {
//debug!("ROUTE TIMER");
        // check out_queue
        let mut to_queue = vec![];
        while let Some(msg) = self.message_queue.dequeue_out() {
            let (wc, addr) = match (msg.message().workchain_id(), msg.message().int_dst_account_id()) {
                (Some(wc), Some(addr)) => (wc, addr),
                _ => continue
            };
            let vals = self.validators_for_account(wc, &addr);

debug!("VALIDATOR SET {:?}", vals);

            // Drop inbound external message if no validators
            if (vals.len() == 0) && msg.message().is_inbound_external() {
                if let Err(res) = self.db.put_message(
                    msg.message().clone(), 
                    MessageProcessingStatus::Refused, 
                    None, None, None)
                {
                    warn!(target: "node", "reflect message reject to DB(1). error: {}", res);
                }
                continue;
            }

            let mut next = 0;
            // Calculate the number validator, which works now
            // while next < self.validators() {
            while next < vals.len() {
                // let next_validator = self.get_next_validator_index(next);
                // let next_step = self.last_step.load(Ordering::SeqCst) + (self.interval() as usize * next);
                let next_step = self.last_step.load(AtomicOrdering::SeqCst) + next + 1;
                let next_validator = self.get_active_on_step(next_step, &vals) as usize;
debug!("TRY ROUTING {}/{} {}:{}:{}", next, next_step, wc, addr, next_validator);
                let route_node = self.routing_table().get_route_by_wc_addr_and_validator(
                    wc, addr.clone(), next_validator
                );
                if let Some(route_node) = route_node {
//debug!(target: "node", "SEND ROUTING MSG {:?}", msg);
debug!(target: "node", "SEND ROUTING MSG {} -> {}", self.validator_index(), next_validator);
                    // send message to "right" node
                    let qm = QueuedMessage::with_route_message(RouteMessage{
                        peer: self.validator_index() as usize,
                        msg: msg.message().clone()
                    }).unwrap();
                    let res = self.send_message_to_node(io, &route_node, qm);
                    // if message accepted - delete it from queue
                    // message accepted only if validator will generate block on next step     
                    if res.is_err() {
                        warn!(target: "node", "Error route message to another node: {}", res.unwrap_err());
                        // select next validator
                        next += 1;
                    } else {
                        break;
                    }
                } else {
                    // if validator not exists - select next validator
                    next += 1;   
                }
            }
 
            if next == vals.len() {
                // if next == self.validators() {
                // if route node not found - return message back
                warn!(target: "node", "No route found for message id {:x}, dst addr {}:{:x}", 
                    msg.message().hash().unwrap(), wc, addr
                );
                to_queue.push(msg);
            }
        }

        for msg in to_queue {
            self.push_message(msg, "Message queue is full after no validator", 10);
        }
    }

    pub fn set_ready_mode_timer(&self) {
        // get index previous validator
        // check step. if step is previous validator
        // start wait GEN_BLOCK_TIMEOUT
        // set ready mode = true
//        let prev_validator = if self.validator_index() != 0 { self.validator_index() - 1 } else { self.validators() as i32 - 1 };
        let step = self.poa_context.lock().engine.get_current_step() as usize;
//println!("STEP!! {} {} {}", step, self.interval(), self.validator_index());
//        if self.is_active_on_step(step + self.interval(), self.current_shard_id(), self.validator_index()) {
        if self.is_active_on_step(step + 1, self.current_shard_id(), self.validator_index()) {
//        if (step % self.validators()) == prev_validator as usize {
            // wait for previous node generate block
            std::thread::sleep(std::time::Duration::from_millis(self.gen_block_timeout()));
            // set ready flag
            self.message_queue.set_ready(true);
            // wait for nodes validate block
            std::thread::sleep(
                std::time::Duration::from_secs(self.interval() as u64) - 
                std::time::Duration::from_millis(self.gen_block_timeout())
            );
            // reset ready flag
            self.message_queue.set_ready(false);
        }
    }

    fn gen_timer(&self, io: &dyn NetworkContext) {
        let peer_list = self.get_network_service().ready_peers();
        match self.get_new_block() {
            BlockData::Block(request) => {
                info!(target: "node", "send block to other peers");
                for p in peer_list.iter() {
                    if let Some(info) = self.routing_table().get_info_by_peer(p) {
                        if info.is_same_shard(self.current_shard_id()) {
                            info!(target: "node", "send block to {}", p);
                            let res = self.send_block_to_validator(io, p, request.clone()); 
                            if res.is_err() {
                                warn!(target: "node", "send block to {} error: {:?}", p, res);
                            }
                        }
                    }
                }
            },
            BlockData::EmptyStep(request) => {
                info!(target: "node", "send empty step to other peers");
                for p in peer_list.iter() {
                    if let Some(info) = self.routing_table().get_info_by_peer(p) {
                        if info.is_same_shard(self.current_shard_id()) {
                            info!(target: "node", "send empty step to {}", p);
                            let res = self.send_empty_step_to_validator(io, p, request.clone());
                            if res.is_err() {
                                warn!(target: "node", "send empty step to {} error: {:?}", p, res);
                            }
                        }
                    }
                }
            }
            _ => (),
        }
    }
    
    fn reset_sync_timer(&self, _io: &dyn NetworkContext) {
        if self.is_synchronize.load(AtomicOrdering::SeqCst) {
            info!(target: "node", "reset_sync_timer value: {}", self.sync_limit.load(AtomicOrdering::SeqCst));
            if self.sync_limit.fetch_add(1, AtomicOrdering::SeqCst) > self.reset_sync_limit() {
                self.stop_sync();
            }
        }
    }

    fn send_message_to_node(&self, io: &dyn NetworkContext, peer: &PeerId, msg: QueuedMessage) -> NodeResult<()> {
        let cell = msg.message().serialize()?;
        let data = serialize_toc(&cell)?;
        let request = networkprotocol::SendMessageRequest {
            id: 0,
            message: ton_api::ton::bytes(data)
        };
        self.send_request(io, peer, &request.into_boxed(), TonNodeEngine::process_send_message_result)
    }

    fn process_send_message_result(&self, _io: &dyn NetworkContext, peer: &PeerId, request: NetworkProtocol) -> NodeResult<()>  {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_SendMessageResponse(_confirmation) => {
                //info!(target: "node", "!!!!! Confirm message send !!!!");
                //info!(target: "node", "result = {}", confirmation.result);

                // TODO check result
                // if error, back message to queue
            }
            NetworkProtocol::TonEngine_NetworkProtocol_Error(error) => {
                // validate and process block
                warn!(target: "node", "Confiramation massage send  from client {} faulure: {}", peer, error.msg);
            }
            _ => return node_err!(NodeErrorKind::TlIncompatiblePacketType)
        }
        Ok(())
    }

    fn send_block_to_validator(&self, io: &dyn NetworkContext, peer: &PeerId, data: Vec<u8>) -> NodeResult<()> {
        let request = networkprotocol::ValidationRequest {
            id: 0,
            signed_block: ton_api::ton::bytes(data)
        };
        self.send_request(io, peer, &request.into_boxed(), TonNodeEngine::process_validate_confirm)
    }

    fn send_empty_step_to_validator(&self, io: &dyn NetworkContext, peer: &PeerId, data: Vec<u8>) -> NodeResult<()> {
        let request = networkprotocol::EmptyStepRequest {
            id: 0,
            empty_step: ton_api::ton::bytes(data)
        };
        self.send_request(io, peer, &request.into_boxed(), TonNodeEngine::process_validate_confirm)
    }

    fn send_block_request(&self, io: &dyn NetworkContext, peer: &PeerId, seq_no: u32, vert_sec_no: u32) -> NodeResult<()> {
        let request = networkprotocol::RequestBlockByNumber {
            id: 0,
            seq_no: seq_no as i32,
            vert_seq_no: vert_sec_no as i32,
        };
        self.send_request(io, peer, &request.into_boxed(), TonNodeEngine::process_sync_block)
    }

    fn verify_block(&self, io: &dyn NetworkContext, peer: &PeerId, request: NetworkProtocol) -> NodeResult<NetworkProtocol> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_ValidationRequest(request) => {
                // validate and process block
                info!(target: "node", "!!!! Validate block !!!!");
                
                let confirmation = self.process_block(&Vec::from(request.signed_block.0), io, peer)?;
                Ok(confirmation.into_boxed())
            }
            _ => node_err!(NodeErrorKind::TlIncompatiblePacketType),

        }
    }

    fn verify_empty_step(&self, io: &dyn NetworkContext, peer: &PeerId, request: NetworkProtocol) -> NodeResult<NetworkProtocol> {
        if let NetworkProtocol::TonEngine_NetworkProtocol_EmptyStepRequest(request) = request {

            info!(target: "node", "!!!! Validate empty step !!!!");

            let confirmation = networkprotocol::ConfirmValidation {
                id: 0,
                peer: peer.clone() as i32,
                result: self.process_empty_step(&Vec::from(request.empty_step.0), io, peer)?,
                block_start_lt: 0,
                block_seq_no: 0,
                block_end_lt: 0,
                block_gen_utime: 0,
            };
            return Ok(confirmation.into_boxed())
        } 
        node_err!(NodeErrorKind::TlIncompatiblePacketType)
    }

    fn process_validate_confirm(&self, io: &dyn NetworkContext, peer: &PeerId, request: NetworkProtocol) -> NodeResult<()>  {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_ConfirmValidation(confirmation) => {
                // process confirmation
                info!(target: "node", "!!!!! Confirm block or empty step !!!!");
                
                self.process_confirmation(*confirmation, io, peer)?;
            }
            NetworkProtocol::TonEngine_NetworkProtocol_Error(error) => {
                // validate and process block
                warn!(target: "node", "Validate block on client {} faulure: {}", peer, error.msg);
            }
            _ => return node_err!(NodeErrorKind::TlIncompatiblePacketType),
        }
        Ok(())
    }

    fn request_block(&self, _io: &dyn NetworkContext, _peer: &PeerId, request: NetworkProtocol) -> NodeResult<NetworkProtocol> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_RequestBlockByNumber(request) => {
                // validate and process block
                info!(target: "node", "!!!! Request block by number !!!!");
                
                return Ok(self.process_block_request(request.seq_no as u32, request.vert_seq_no as u32)?.into_boxed())
            }
            _ => node_err!(NodeErrorKind::TlIncompatiblePacketType)

        }
    }

    fn process_sync_block(&self, io: &dyn NetworkContext, peer: &PeerId, request: NetworkProtocol) -> NodeResult<()> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_ResponceBlock(block) => {
                // process block
                info!(target: "node", "!!!! Process sync block !!!!");
                self.sync_limit.store(0, AtomicOrdering::SeqCst);
                //self.process_confirmation(*confirmation, io, peer)?;
                if let Ok((mut seq_no, vert_seq_no)) = self.process_block_sync(&block.signed_block.0) {
                    seq_no += 1;
                    if !self.incoming_blocks.exists(seq_no) {
                        if self.send_block_request(io, peer, seq_no, vert_seq_no).is_err() {
                            self.stop_sync();
                        }
                    } else {
                        self.finalyze_sync();
                    }
                } else {
                    self.stop_sync();
                }
            } 
            NetworkProtocol::TonEngine_NetworkProtocol_Error(error) => {
                // validate and process block
                warn!(target: "node", "Get block by seq_no FAILED: {}", error.msg);
            }
            _ => return node_err!(NodeErrorKind::TlIncompatiblePacketType)
        }
        Ok(())
    }

    fn process_request_node_info(&self, _io: &dyn NetworkContext, _peer: &PeerId, request: NetworkProtocol) -> NodeResult<NetworkProtocol> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_RequestNodeInfo(_request) => {
                // get node info
                info!(target: "node", "!!!! Request node info !!!!");
                
                return Ok(self.process_node_info_request()?.into_boxed())
            }
            _ => node_err!(NodeErrorKind::TlIncompatiblePacketType)

        }
    }

    fn process_request_send_message(&self, _io: &dyn NetworkContext, _peer: &PeerId, request: NetworkProtocol) -> NodeResult<NetworkProtocol> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_SendMessageRequest(request) => {
                //info!(target: "node", "!!!! Send Message Request !!!!");
                
                let msg = Message::construct_from_bytes(&request.message.0)?;
                let msg = QueuedMessage::with_message(msg).unwrap();
                
                return Ok(self.process_send_message_request(msg)?.into_boxed())
            }
            _ => node_err!(NodeErrorKind::TlIncompatiblePacketType)

        }
    }

    pub fn reflect_transaction_to_db(
        db: Arc<Box<dyn DocumentsDb>>,
        transaction: Transaction,
        account: Option<Account>, workchain_id: i32) -> NodeResult<()> {

        let transaction_id = transaction.hash().ok();
        if let Ok(Some(in_msg)) = transaction.read_in_msg() {
            let _res = db.put_message(
                in_msg.clone(), 
                MessageProcessingStatus::Preliminary, 
                transaction_id.clone(), Some(transaction.now()), None
            )?;
        }

        transaction.iterate_out_msgs(&mut |msg| {
            db.put_message(
                msg, 
                MessageProcessingStatus::Preliminary, 
                transaction_id.clone(), None, None
            ).map_err(|_| failure::format_err!("put_to_db error"))?;
            Ok(true)
        })?;

        db.put_transaction(
            transaction, 
            TransactionProcessingStatus::Preliminary, 
            None,
            workchain_id
        )?;

        if let Some(account) = account {
            db.put_account(account)?;
        }
        
        Ok(())
    }

    fn process_request_reflect_to_db(&self, _io: &dyn NetworkContext, _peer: &PeerId, request: NetworkProtocol) -> NodeResult<NetworkProtocol> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_ReflectToDbRequest(request) => {
                //info!(target: "node", "!!!! Send Reflect to DB Request !!!!");
                
                let transaction = Transaction::construct_from_bytes(&request.transaction.0)?;
                let mut workchain_id = 0;
                let mut acc = None;
                if !request.account.0.is_empty() {
                    let account = Account::construct_from_bytes(&request.account.0)?;
                    workchain_id = account.get_addr().map(|addr| addr.get_workchain_id()).unwrap_or_default();
                    acc = Some(account);
                }

                // TODO write transaction to DB with prelimitary state
                Self::reflect_transaction_to_db(self.db.clone(), transaction, acc, workchain_id)?;
            }
            _ => return node_err!(NodeErrorKind::TlIncompatiblePacketType),

        };
        Ok(networkprotocol::ReflectToDbResponse{
            id: 0,
            result: 0,
        }.into_boxed())
    }
}


impl TonNodeEngine {

    ///
    /// Process pequest for node routing info
    /// 
    fn process_node_info_request(&self) -> NodeResult<networkprotocol::ResponseNodeInfo> {
        let shard = self.current_shard_id();
        let info = networkprotocol::ResponseNodeInfo {
            id: 0,
            validator_no: self.validator_index(),
            workchain: shard.workchain_id(),
            shard_prefix: shard.shard_prefix_without_tag() as i64,
            shard_pfx_len: shard.prefix_len() as i32,
        };
        Ok(info)
    }


    ///
    /// Process request for routing message received
    /// 
    fn process_send_message_request(&self, msg: QueuedMessage) -> NodeResult<networkprotocol::SendMessageResponse> {

        // TODO may be        
        // check if message for this node (workchain, shardchain)
        // check validator step - otherwise return error

        let res = self.message_queue.queue(msg);
        if res.is_err() {
            warn!(target: "node", "process_send_message_request queue error: queue is full");
            return Ok(networkprotocol::SendMessageResponse{
                id: 0,
                result: -1,
            })
        }
        Ok(networkprotocol::SendMessageResponse{
            id: 0,
            result: 0,
        })
    }


    ///
    /// Call generate new block if validator time is right
    /// 
    fn get_new_block(&self) -> BlockData {
debug!("GETNEW");
        let step = self.poa_context.lock().engine.get_current_step() as usize;
        let self_last_step = self.last_step.load(AtomicOrdering::SeqCst);
        if self_last_step == step {
            debug!(target: "node", "step not change");
            BlockData::None
        } else if self.is_active_on_step(step, self.current_shard_id(), self.validator_index())
//            && ((step % self.validators()) == self.validator_index() as usize)
        {
            if self.is_self_block_processing.load(AtomicOrdering::SeqCst) {
                warn!(target: "node", "Block is processing");
                return BlockData::None
            }
            self.last_step.store(step, AtomicOrdering::SeqCst);
            match self.prepare_block(step as u32) {
                Err(err) => {
                    warn!(target: "node", "Error in prepare block: {:?}", err);
                    self.is_self_block_processing.store(false, AtomicOrdering::SeqCst);
                    BlockData::None
                }
                Ok(BlockData::Block(data)) => {
                    debug!(target: "node", "new Block");
                    BlockData::Block(data)
                }
                Ok(BlockData::EmptyStep(data)) => {
                    debug!(target: "node", "empty steps");
                    BlockData::EmptyStep(data)
                }
                Ok(BlockData::None) => {
                    debug!(target: "node", "Block is none");
                    BlockData::None
                }
            }
        } else {
            warn!(target: "node", "cannot prepare block");
            warn!(target: "node", "poa step context: {}, self last step: {}", step, self_last_step);
            warn!(target: "node", "current shard id: {}, validator index: {}", self.current_shard_id(), self.validator_index());
            warn!(target: "node", "is_active_on_step: {}", self.is_active_on_step(step, self.current_shard_id(), self.validator_index()));
            BlockData::None
        }
    }

    pub fn print_block_info(block: &Block) {
        let extra = block.read_extra().unwrap();
        info!(target: "node", 
            "block: gen time = {}, in msg count = {}, out msg count = {}, account_blocks = {}",
            block.read_info().unwrap().gen_utime(),
            extra.read_in_msg_descr().unwrap().len().unwrap(),
            extra.read_out_msg_descr().unwrap().len().unwrap(),
            extra.read_account_blocks().unwrap().len().unwrap());
    }

    /*fn propose_block_to_db(&self, block_candidate: &mut Block) -> NodeResult<()> {
                        // save objects into kafka with "proposed" state
        block_candidate.extra.in_msg_descr.iterate(&mut |mut in_msg| {
            if let Some(msg) = in_msg.message_mut() {
                self.db.put_message(
                    msg, 
                    MessageProcessingStatus::Proposed, 
                    Some(block_candidate.id.clone())
                ).unwrap();
            }
            Ok(true)
        })?;

        block_candidate.extra.out_msg_descr.iterate(&mut |mut out_msg| {
            if let Some(msg) = out_msg.message_mut() {
                self.db.put_message(
                    msg, 
                    MessageProcessingStatus::Proposed, 
                    Some(block_candidate.id.clone())
                ).unwrap();
            }
            Ok(true)
        })?;

        block_candidate.extra.account_blocks.iterate(&mut |account_block| {
            account_block.transaction_iterate(&mut |mut transaction| {
                self.db.put_transaction(
                    &mut transaction, 
                    TransactionProcessingStatus::Proposed, 
                    Some(block_candidate.id.clone()),
                    block_candidate.info.gen_utime.clone()
                ).unwrap();
                Ok(true)
            }).unwrap();
            Ok(true)
        })?;

        self.db.put_block(block_candidate, BlockProcessingStatus::Proposed)?;
        
        Ok(())
    }*/

    ///
    /// Generate new block if possible
    /// 
    pub fn prepare_block(&self, timestamp: u32) -> NodeResult<BlockData> {

        if !self.poa_context.lock().engine.can_propose() {
            // Cannot propose due to PoA schedule
            warn!(target: "node", "");
            warn!(target: "node", "PoA schedule prohibits to gen block right now");
            warn!(target: "node", "");
            return Ok(BlockData::None);
        }

let mut now = Instant::now();
let mut time = [0u128; 10];
        let mut block = Block::default();
        let mut new_shard_state: Option<ShardStateUnsplit> = None;

        {
debug!("PREP_BLK1");
            if !self.is_synchronize.load(AtomicOrdering::SeqCst) {
                let blk_prev_info = self.finalizer.lock().get_last_block_info()?;
                let mut info = block.read_info()?;
                info.set_seq_no(self.finalizer.lock().get_last_seq_no() + 1)?;
                info.set_prev_stuff(false, &blk_prev_info)?;
                info.set_gen_utime(UnixTime32(timestamp));
                block.write_info(&info)?;
debug!("PREP_BLK");

                if !self.is_self_block_processing.load(AtomicOrdering::SeqCst)
                    && !self.is_starting.load(AtomicOrdering::SeqCst)
                {
                    self.is_self_block_processing.store(true, AtomicOrdering::SeqCst);
debug!("PREP_BLK_START");                        
                    let finalizer = self.finalizer.lock();
                    let shard_state = finalizer.get_last_shard_state();                    
                    let gen_block_time = Duration::from_millis(self.gen_block_timeout());
time[0] = now.elapsed().as_micros();
now = Instant::now();
                    
                    let seq_no = block.read_info()?.seq_no();
                    if let Some((block_candidate, Some(new_shard))) = 
                        self.msg_processor.lock().generate_block_multi(
                        &shard_state, 
                        gen_block_time,
                        seq_no, 
                        blk_prev_info,
                        timestamp, 
                        true //tvm code tracing enabled by default on trace level
                    )? {

time[1] = now.elapsed().as_micros();
now = Instant::now();
                        
                        // reset ready-mode for in-queue
//                        self.message_queue.set_ready(false);

                        block = block_candidate;
                        new_shard_state = Some(new_shard);

                        let mut ca = ConfirmationAnalizer::new(
                            self.validators(self.current_shard_id()).len() as i32
                        );
                        ca.block_candidate = block.clone();

time[2] = now.elapsed().as_micros();
now = Instant::now();

                        ca.confirmations.push(networkprotocol::ConfirmValidation {
                            id: 0,
                            peer: 0, //self
                            result: 0,
                            block_seq_no: seq_no as i32,
                            block_start_lt: block.read_info()?.start_lt() as i64,
                            block_end_lt: block.read_info()?.end_lt() as i64,
                            block_gen_utime: block.read_info()?.gen_utime().0 as i32,
                        });
                        self.analyzers.lock().clear();
                        self.analyzers.lock().insert(seq_no, ca);

time[3] = now.elapsed().as_micros();
now = Instant::now();

                    }
debug!("PREP_BLK_AFTER_GEN");
                } else {
                    //engine is busy, pass gen block
                    warn!(target: "node", "");
                    warn!(target: "node", "engine is busy, pass gen block");
                    warn!(target: "node", "self_block_processing = {}, is_starting = {}.",
                        self.is_self_block_processing.load(AtomicOrdering::SeqCst),
                        self.is_starting.load(AtomicOrdering::SeqCst)
                    );
                    warn!(target: "node", "");
                    if self.is_self_block_processing.load(AtomicOrdering::SeqCst) {
                        return Ok(BlockData::None);
                    }
                }
            } else {
                // synchronize in process
                warn!(target: "node", "");
                warn!(target: "node", "synchronize in process. pass gen block");
                warn!(target: "node", "");
                return Ok(BlockData::None);
            }
debug!("PREP_BLK2");
        }
        
//        self.message_queue.set_ready(false);
        
        // TODO remove debug print
        Self::print_block_info(&block);

        /*let res = self.propose_block_to_db(&mut block);

        if res.is_err() {
            warn!(target: "node", "Error propose_block_to_db: {}", res.unwrap_err());
        }*/

        let parent = OrdinaryHeader::default();
        let mut exc_block = ExecutedBlock::new(Arc::new(Vec::new()));
        exc_block.header.set_number(block.read_info()?.seq_no() as u64);
        // let prev_ref_hash = block.read_info()?.prev_ref_cell().repr_hash();
        let prev_ref_cell = block.read_info()?.read_prev_ref()?.serialize()?;
        let prev_ref_hash = prev_ref_cell.repr_hash();
        info!(target: "node", "PARENT block hash: {:?}", prev_ref_hash);
        exc_block.header.set_parent_hash(H256::from(prev_ref_hash.as_slice()));
        exc_block.header.ton_block = TONBlock::Unsigned(UnsignedBlock {
            block: block,
            keyid: self.poa_context.lock().engine.key_id(),
        });
        
time[4] = now.elapsed().as_micros();
now = Instant::now();

        let (block_data, seal_details) = match self.poa_context.lock().engine.generate_seal(
            &exc_block, 
            &parent
        ) {
            (Seal::Regular(mut fields), details) => {
                if !fields[1].is_empty() {
                    (BlockData::Block(fields.remove(1)), details)
                } else {
                    (BlockData::EmptyStep(fields.remove(2)), details)
                }
/*
                let bytes = fields[1].clone();
                if !bytes.is_empty() {
                    hexdump(&bytes[..if bytes.len() > 100 { 100 } else { bytes.len() }]);
                    BlockData::Block(bytes)
                } else {
                    let bytes = fields[2].clone();
                    hexdump(&bytes[..if bytes.len() > 100 { 100 } else { bytes.len() }]);
                    BlockData::EmptyStep(bytes)
                }
*/
            }
            _ => (BlockData::None, Vec::new())
        };

time[5] = now.elapsed().as_micros();
now = Instant::now();
        
        if let BlockData::Block(ref data) = block_data {
            let mut block_rdr = Cursor::new(&data);
            let s_block = SignedBlock::read_from(&mut block_rdr)?;
time[6] = now.elapsed().as_micros();
now = Instant::now();
            let (_, details) = self.finality_and_apply_block(
                s_block, 
                &block_rdr.into_inner(), 
                new_shard_state, false
            )?;

time[7] = now.elapsed().as_micros();
info!(target: "profiler", 
    "{} {} / {} / {} / {} / {} / {} micros",
    "Prepare block time: setup/gen/analysis1/analysis2/seal/finality",
    time[0], time[1], time[2], time[3], time[4] + time[5], time[6] + time[7]
);

let mut str_details = String::new();
for detail in seal_details {
    str_details = if str_details.is_empty() {
        format!("{}", detail)
    } else {
        format!("{} / {}", str_details, detail)
    }
}
info!(target: "profiler", "Block sealing details: {} / {} micros",  time[4], str_details);

let mut str_details = String::new();
for detail in details {
    str_details = if str_details.is_empty() {
        format!("{}", detail)
    } else {
        format!("{} / {}", str_details, detail)
    }
}
info!(target: "profiler", "Block finality details: {} / {} micros",  time[6], str_details);

        }

        self.is_self_block_processing
            .store(false, AtomicOrdering::SeqCst);
debug!("PREP_BLK_SELF_STOP");

        Ok(block_data) 

    }

    fn call_poa_finality(&self, block: SignedBlock) 
        -> NodeResult<(OrdinaryHeader, Vec<UInt256>, Vec<u128>)> 
    {

let mut time = Vec::new();
let now = Instant::now();

        info!(target: "node", "Block finality check:\n");
        let mut header = OrdinaryHeader::default();
        let hashes;
        header.ton_block = TONBlock::Signed(block);

        if !self.is_local() { 
            let result = self.poa_context.lock().engine.verify_block_external(&header);
            if result.is_err() {
                return node_err!(NodeErrorKind::ValidationBlockError);
            }
        }

time.push(now.elapsed().as_micros());
let now = Instant::now();

        if let TONBlock::Signed(ref block) = header.ton_block {
            let mut signers = Vec::new();
            for key in block.signatures().keys() {
                signers.push(*key)
            }
debug!("SIGNERS {:?}", signers);
            let prepared: H256 = block.block_hash().as_slice().into();
            let finalized = self.finalizer.lock().finality.push_hash(prepared, signers).unwrap();
            hashes = finalized.iter().map(|h| UInt256::from(h.0)).collect();
            info!(target: "node", "prepared: {:x?}\nfinalized: {:x?}", 
                hex::encode(prepared.0), hashes);
            
        } else {
            return node_err!(NodeErrorKind::FinalityError);
        }

time.push(now.elapsed().as_micros());

        info!(target: "node", "Block finality check: OK\n");
        return Ok((header, hashes, time));

    }

    fn process_empty_step(&self, data: &[u8], io: &dyn NetworkContext, peer: &PeerId) -> NodeResult<i64> {
        
        let res = self.poa_context.lock().engine.ton_handle_message(data);
        if res.is_err() {
            return node_err!(NodeErrorKind::ValidationEmptyStepError);
        }

        let (ep, key) = res.unwrap();

        let expected_seq_no = self.finalizer.lock().get_last_seq_no() + 1;
        info!(target: "node", "Expected seq_no = {} vs {}, empty step", expected_seq_no, ep.next_seq_no);

        if ep.next_seq_no > expected_seq_no {
            warn!(target: "node", "Empty block with greater sequense number. Our node should synchronize.");
            self.request_for_synchronize(io, peer)?;
            return Ok(4);
        } else if ep.next_seq_no < expected_seq_no{
            warn!(target: "node", "Empty block with less sequense number. Other node should synchronize.");                
            return Ok(5);
        }

        if !self.is_synchronize.load(AtomicOrdering::SeqCst) {
            let mut signers = Vec::new();
            signers.push(key);

            let prepared: H256 = ep.parent_hash;
            let mut finalizer = self.finalizer.lock();
            match finalizer.finality.push_hash(prepared, signers) {
                Ok(finalized) => {
                    let hashes = finalized.iter().map(|h| UInt256::from(h.0)).collect();
                    info!(target: "node", "EP:\nprepared: {:x?}\nfinalized: {:x?}", prepared, finalized);
                    finalizer.finalize_without_new_block(hashes)?;
                },
                Err(err) => {
                    warn!(target: "node", "Finalization (empty step) error {:?}", err);                
                    panic!("Finalization (empty step) error {:?}", err);                
                }
            }

            // reset starting flag
            self.is_starting.store(false, AtomicOrdering::SeqCst);

        }

        // TODO remove
        info!(target: "node", "Empty block - skipped");
        Ok(3)
    }

    fn process_block(&self, data: &[u8], io: &dyn NetworkContext, peer: &PeerId ) 
        -> NodeResult<networkprotocol::ConfirmValidation> {
        let mut cc = networkprotocol::ConfirmValidation::default();

        let s_block = SignedBlock::read_from(&mut Cursor::new(&data)).unwrap();
        let expected_seq_no = self.finalizer.lock().get_last_seq_no() + 1;
        info!(target: "node", "Expected seq_no = {}, block", expected_seq_no);

        cc.result = 0; // OK
        cc.block_seq_no = s_block.block().read_info()?.seq_no() as i32;
        cc.block_start_lt = s_block.block().read_info()?.start_lt() as i64;
        cc.block_end_lt = s_block.block().read_info()?.end_lt() as i64;
        cc.block_gen_utime = s_block.block().read_info()?.gen_utime().0 as i32;

        if !self.is_synchronize.load(AtomicOrdering::SeqCst)
            && !self.is_self_block_processing.load(AtomicOrdering::SeqCst)
        {
            if s_block.block().read_info()?.seq_no() > expected_seq_no {
                warn!(target: "node", "Block with greater sequense number. Our node should synchronize.");
                cc.result = 4;
                // need synchronize self
                self.request_for_synchronize(io, peer)?;
                return Ok(cc);
            } else if s_block.block().read_info()?.seq_no() < expected_seq_no {
                warn!(target: "node", "Block with less sequense number. Other node should synchronize.");
                cc.result = 5;
                return Ok(cc);
            }

            let res = self.finality_and_apply_block(
                s_block,
                data,
                None,
                false,
            );

            if res.is_err() {
                warn!(target: "node", "!!! received block not applied !!!\n{:?}", res.unwrap_err());
                cc.result = 2; // block inapplicable
                self.request_for_synchronize(io, peer)?;
            } else {
                info!(target: "node", "!!! received block applied successfully !!!");
            }
            
            // reset starting flag
            self.is_starting.store(false, AtomicOrdering::SeqCst);

        } else {
            // is synchronize
            info!(target: "node", "Save block to incomming block cache");
            self.incoming_blocks.push(FinalityBlockInfo::with_block(
                s_block,
                Some(data.to_vec()),
                vec![],
            ));
        }

        Ok(cc)
    }

    fn process_confirmation(&self, c: networkprotocol::ConfirmValidation, io: &dyn NetworkContext, _peer: &PeerId) -> NodeResult<()> {
        
        match c.result {
            0 => info!(target: "node", "process_confirmation result: Block applied."),
            1 => warn!(target: "node", "process_confirmation result: Block verify check log::error!"),
            2 => warn!(target: "node", "process_confirmation result: Applay block to ShardState failed!"),
            3 => info!(target: "node", "process_confirmation result: Empty step applied."),
            4 => warn!(target: "node", "process_confirmation result: Block with greater seq_no"),
            5 => warn!(target: "node", "process_confirmation result: Block with less seq_no"),
            _ => warn!(target: "node", "process_confirmation result: unknown"),
        };

        let seq_no = c.block_seq_no as u32;

        // if empty step - return
        if c.result == 3 {
            return Ok(());
        }
/*
        if (c.result == 5) && (seq_no == 0) {
            self.request_for_synchronize(io, peer)?;
            return Ok(());
        }
*/

        info!(target: "node", "Block info:");
        info!(target: "node", "\tseq_no: {}", c.block_seq_no);
        info!(target: "node", "\tstart_lt: {}", c.block_start_lt);
        info!(target: "node", "\tend_lt: {}", c.block_end_lt);
        info!(target: "node", "\tgen_utime: {}", c.block_gen_utime);

        info!(target: "node", "analysers {:?}", self.analyzers.lock().keys());
        let mut analyzers = self.analyzers.lock();
        let mut completed = false;

        if let Some(analyzer) = analyzers.get_mut(&seq_no) {
            analyzer.confirmations.push(c);
            if analyzer.select_actual_validators().is_some() {
                // self block applied
                self.is_self_block_processing.store(false, AtomicOrdering::SeqCst);
                completed = true;
            } else if analyzer.select_failed_validators().is_some() {
                // discard block - sync from someone
                warn!(target: "node", "!!! Self block discarded !!! Need synchronization !!!");
                if let Some(peers) = analyzer.select_failed_validators() {
                    self.request_for_synchronize(io, &peers[0])?;
                }
                // self block applied or discarded
                self.is_self_block_processing.store(false, AtomicOrdering::SeqCst);
                completed = true;
            }
        } else {
            info!(target: "node", "Unknown confirmation or processed yet");
        }
        if completed {
            analyzers.remove(&seq_no);
        }
        Ok(())
    }


    fn request_for_synchronize(&self, io: &dyn NetworkContext, peer: &PeerId) -> NodeResult<()> {
        if self.start_sync() {
            // get last finality block seq no
            let (last_finalized_seq_no, _last_finalized_shard_hash) = self
                .finalizer.lock().get_last_finality_shard_hash()?;

            self.reset_finality()?;

            // start sync from sec_no + 1
            let seq_no =  1 + last_finalized_seq_no as u32;
            let vert_seq_no = (last_finalized_seq_no >> 32) as u32;

            info!(target: "node", "seng request for block with number: {}, {}", seq_no, vert_seq_no);

            if !self.incoming_blocks.exists(seq_no) {
                if self.send_block_request(io, peer, seq_no, vert_seq_no).is_err() {
                    self.stop_sync();
                }
            } else {
                self.finalyze_sync();
            }
        }
        Ok(())
    }

    fn reset_finality(&self) -> NodeResult<()> {
        info!(target: "node", "Reset finality!");
        self.finalizer.lock().reset()?;
        self.finalizer.lock().finality.clear();
        self.finalizer.lock().save_finality()?;
        Ok(())
    }

    fn process_block_request(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<networkprotocol::ResponceBlock> {
        // get block with specified number and send to peer
        let block_data = self.finalizer.lock().get_raw_block_by_seqno(seq_no, vert_seq_no);

        if block_data.is_ok() {
            return Ok(networkprotocol::ResponceBlock {
                id: 0,
                signed_block: ton_api::ton::bytes(block_data.unwrap()),
            })
        } else {
            match block_data.unwrap_err() {
                NodeError(NodeErrorKind::Io(err), _) => {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        return Ok(networkprotocol::ResponceBlock {
                            id: 0,
                            signed_block: ton_api::ton::bytes(vec![255u8, 0xFF, 0xFF, 0xFF]),
                        })
                    } else {
                        bail!(err);    
                    }
                },
                err => {
                    warn!(target: "node", "Get serialized block failed: {:?}", err);
                    bail!(err);
                }
            }
        }
    }

    /// finalize synchronization
    /// Apply all block accumulated until sync and stop
    fn finalyze_sync(&self) {
        self.apply_incoming_blocks();
        self.is_synchronize.store(false, AtomicOrdering::SeqCst);
    }

    fn apply_incoming_blocks(&self) {
        
        self.incoming_blocks.sort();
        info!(target: "node", "Applay incoming blocks");

        loop {
            if self.incoming_blocks.len() > 0 {
                let fblock_info = self.incoming_blocks.remove(0);
                let expected_seq_no = self.finalizer.lock().get_last_seq_no() + 1;
                let block_seq_no = fblock_info.block.block().read_info().unwrap().seq_no();

                if block_seq_no == expected_seq_no {

                    let res = self.finality_and_apply_block(
                        fblock_info.block,
                        &fblock_info.block_data.unwrap(),
                        None,
                        false,
                    );

                    if res.is_ok() {
                        info!(target: "node", "incoming block seq_no = {} applied", block_seq_no);
                    } else {
                        warn!(target: "node", "temporary block seq_no = {} applay error {:?}!",
                            block_seq_no, res
                        );
                    }
                }
            } else {
                break;
            }
        }
    }

    ///  stop synchronization
    fn stop_sync(&self) {
        info!(target: "node", "Stop synchronization!");
        self.is_synchronize.store(false, AtomicOrdering::SeqCst);
        self.sync_limit.store(0, AtomicOrdering::SeqCst);
    }

    ///  stop synchronization
    fn start_sync(&self) -> bool {
        let res = self.is_synchronize.compare_exchange(false, true, AtomicOrdering::SeqCst, AtomicOrdering::SeqCst);
        info!(target: "node", "Start synchronization = {}!", res.is_ok());
        res.is_ok()
    }

    /// finality and apply block
    fn finality_and_apply_block(
        &self, 
        block: SignedBlock, 
        block_data: &[u8],
        applied_shard: Option<ShardStateUnsplit>,
        is_sync: bool,
    ) -> NodeResult<(
        Arc<ShardStateUnsplit>, 
        Vec<u128>
    )> {

        let res = self.call_poa_finality(block);

let now = Instant::now();

        if res.is_ok() {
            let (header, finality_hashes, mut time) = res.unwrap();
            if let TONBlock::Signed(block) = header.ton_block {
                let res = self.block_applier.lock().apply(
                    block, 
                    Some(block_data.to_vec()), 
                    finality_hashes,
                    applied_shard,
                    is_sync,
                );

time.push(now.elapsed().as_micros());
let now = Instant::now();

                if res.is_err() {
                    self.finalizer.lock().finality.remove_last();
                }
                self.finalizer.lock().save_rolling_finality()?;

time.push(now.elapsed().as_micros());

                if res.is_err() {
                    Err(res.err().unwrap())
                } else {
                    let res = res.ok().unwrap();
                    Ok((res, time))
                }
            } else {
                node_err!(NodeErrorKind::FinalityError)
            }
        } else {
            Err(res.unwrap_err())
        }

    }

    ///
    /// process sync block and return current block seq_no
    /// 
    fn process_block_sync(&self, data: &[u8]) -> NodeResult<(u32, u32)> {
        // receive block and applay it to shard state
        // only if synchronization in progress
        if self.is_synchronize.load(AtomicOrdering::SeqCst)
            && !self.is_self_block_processing.load(AtomicOrdering::SeqCst)
        {

            let vec = data.to_vec();
            if vec.len() == 0 {
                info!(target: "node", "Received empty buffer. Sync ended.");
                self.finalyze_sync();
                return node_err!(NodeErrorKind::SynchronizeEnded);
            } 

            if vec == vec![255u8, 0xFF, 0xFF, 0xFF] {
                info!(target: "node", "Received sync end.");
                self.finalyze_sync();
                return node_err!(NodeErrorKind::SynchronizeEnded);
            }

            let block = SignedBlock::read_from(&mut Cursor::new(&data))?;

            let block_seq_no = block.block().read_info()?.seq_no();
            let block_vert_seq_no = block.block().read_info()?.vert_seq_no();

            let res = self.finality_and_apply_block(block, data, None, true);

            if res.is_err() {
                warn!(target: "node", "Sync Block NOT APPLIED! block_seq_no = {}, err = {:?}",
                        block_seq_no, res
                );
                return node_err!(NodeErrorKind::SynchronizeError);
            } else {
                info!(target: "node", "!!! Received block SUCCESSFULLY APPLIED !!!");
                return Ok((block_seq_no, block_vert_seq_no));
            }
        }
        node_err!(NodeErrorKind::SynchronizeInProcess)
    }
}