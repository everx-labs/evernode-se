use super::*;
use crate::error::NodeError;
use std::{io::Cursor, sync::atomic::Ordering as AtomicOrdering, time::Instant};
use ton_api::{
    ton::ton_engine::{network_protocol::*, NetworkProtocol},
    IntoBoxed,
};
use ton_block::Deserializable;
use ton_types::HashmapType;
type PeerId = u64;

pub fn init_ton_node_handlers(ton: &TonNodeEngine) {
    ton.register_timer(
        Duration::from_millis(ton.gen_block_timer()),
        TonNodeEngine::route_message_timer,
    );

    let request = networkprotocol::SendMessageRequest::default();
    ton.register_response_callback(
        request.into_boxed(),
        TonNodeEngine::process_request_send_message,
    );
}

impl TonNodeEngine {
    /*
        fn get_next_validator_index(&self, step_count: usize) -> usize {
            let next_step = self.last_step.load(AtomicOrdering::SeqCst) + (self.interval() as usize * step_count);
            next_step % self.validators()
        }
    */

    fn route_message_timer(&self) {
        //debug!("ROUTE TIMER");
        // check out_queue
        let mut to_queue = vec![];
        while let Some(msg) = self.message_queue.dequeue_out() {
            let (wc, addr) = match (
                msg.message().workchain_id(),
                msg.message().int_dst_account_id(),
            ) {
                (Some(wc), Some(addr)) => (wc, addr),
                _ => continue,
            };
            let vals = self.validators_for_account(wc, &addr);

            debug!("VALIDATOR SET {:?}", vals);

            // Drop inbound external message if no validators
            if (vals.len() == 0) && msg.message().is_inbound_external() {
                continue;
            }

        }

        for msg in to_queue {
            self.push_message(msg, "Message queue is full after no validator", 10);
        }
    }

    fn process_send_message_result(
        &self,
        peer: &PeerId,
        request: NetworkProtocol,
    ) -> NodeResult<()> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_SendMessageResponse(_confirmation) => {
                //info!(target: "node", "!!!!! Confirm message send !!!!");
                //info!(target: "node", "result = {}", confirmation.result);

                // TODO check result
                // if error, back message to queue
            }
            NetworkProtocol::TonEngine_NetworkProtocol_Error(error) => {
                // validate and process block
                warn!(target: "node", "Confirmation massage send  from client {} failure: {}", peer, error.msg);
            }
            _ => return Err(NodeError::TlIncompatiblePacketType),
        }
        Ok(())
    }

    fn process_request_send_message(
        &self,
        _peer: &PeerId,
        request: NetworkProtocol,
    ) -> NodeResult<NetworkProtocol> {
        match request {
            NetworkProtocol::TonEngine_NetworkProtocol_SendMessageRequest(request) => {
                //info!(target: "node", "!!!! Send Message Request !!!!");

                let msg = Message::construct_from_bytes(&request.message.0)?;
                let msg = QueuedMessage::with_message(msg).unwrap();

                return Ok(self.process_send_message_request(msg)?.into_boxed());
            }
            _ => Err(NodeError::TlIncompatiblePacketType),
        }
    }
}

impl TonNodeEngine {
    ///
    /// Process request for node routing info
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
    fn process_send_message_request(
        &self,
        msg: QueuedMessage,
    ) -> NodeResult<networkprotocol::SendMessageResponse> {
        // TODO may be
        // check if message for this node (workchain, shardchain)
        // check validator step - otherwise return error

        let res = self.message_queue.queue(msg);
        if res.is_err() {
            warn!(target: "node", "process_send_message_request queue error: queue is full");
            return Ok(networkprotocol::SendMessageResponse { id: 0, result: -1 });
        }
        Ok(networkprotocol::SendMessageResponse { id: 0, result: 0 })
    }

    ///
    /// Call generate new block if validator time is right
    ///
    fn get_new_block(&self) -> Option<SignedBlock> {
        debug!("GET-NEW");
        let step = self.last_step.fetch_add(1, AtomicOrdering::SeqCst);
        match self.prepare_block(step as u32) {
            Err(err) => {
                warn!(target: "node", "Error in prepare block: {:?}", err);
                None
            }
            Ok(Some(data)) => {
                debug!(target: "node", "new Block");
                Some(data)
            }
            Ok(None) => {
                debug!(target: "node", "no new Block");
                None
            }
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
    pub fn prepare_block(&self, timestamp: u32) -> NodeResult<Option<SignedBlock>> {

        let mut now = Instant::now();
        let mut time = [0u128; 10];
        let mut block = Block::default();
        let mut new_shard_state: Option<ShardStateUnsplit> = None;

        debug!("PREP_BLK1");
        let blk_prev_info = self.finalizer.lock().get_last_block_info()?;
        let mut info = block.read_info()?;
        info.set_seq_no(self.finalizer.lock().get_last_seq_no() + 1)?;
        info.set_prev_stuff(false, &blk_prev_info)?;
        info.set_gen_utime(timestamp.into());
        block.write_info(&info)?;
        debug!("PREP_BLK");

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
                true, //tvm code tracing enabled by default on trace level
            )?
        {
            time[1] = now.elapsed().as_micros();
            now = Instant::now();

            // reset ready-mode for in-queue
            //                        self.message_queue.set_ready(false);

            block = block_candidate;
            new_shard_state = Some(new_shard);

            time[2] = now.elapsed().as_micros();
            now = Instant::now();
            time[3] = now.elapsed().as_micros();
            now = Instant::now();
        }
        debug!("PREP_BLK_AFTER_GEN");
        debug!("PREP_BLK2");

        //        self.message_queue.set_ready(false);

        // TODO remove debug print
        Self::print_block_info(&block);

        /*let res = self.propose_block_to_db(&mut block);

        if res.is_err() {
            warn!(target: "node", "Error propose_block_to_db: {}", res.unwrap_err());
        }*/

        // let prev_ref_hash = block.read_info()?.prev_ref_cell().repr_hash();
        let prev_ref_cell = block.read_info()?.read_prev_ref()?.serialize()?;
        let prev_ref_hash = prev_ref_cell.repr_hash();
        info!(target: "node", "PARENT block hash: {:?}", prev_ref_hash);

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
        info!(target: "profiler",
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
        info!(target: "profiler", "Block finality details: {} / {} micros",  time[6], str_details);

        debug!("PREP_BLK_SELF_STOP");

        Ok(Some(s_block))
    }

    fn process_block(
        &self,
        data: &[u8],
        peer: &PeerId,
    ) -> NodeResult<()> {

        let s_block = SignedBlock::read_from(&mut Cursor::new(&data)).unwrap();
        let expected_seq_no = self.finalizer.lock().get_last_seq_no() + 1;
        info!(target: "node", "Expected seq_no = {}, block", expected_seq_no);

        let res = self.finality_and_apply_block(&s_block, data, None, false);

        if res.is_err() {
            warn!(target: "node", "!!! received block not applied !!!\n{:?}", res.unwrap_err());
        } else {
            info!(target: "node", "!!! received block applied successfully !!!");
        }

        Ok(())
    }

    fn apply_incoming_blocks(&self) {
        self.incoming_blocks.sort();
        info!(target: "node", "Apply incoming blocks");

        loop {
            if self.incoming_blocks.len() > 0 {
                let finality_block_info = self.incoming_blocks.remove(0);
                let expected_seq_no = self.finalizer.lock().get_last_seq_no() + 1;
                let block_seq_no = finality_block_info
                    .block
                    .block()
                    .read_info()
                    .unwrap()
                    .seq_no();

                if block_seq_no == expected_seq_no {
                    let res = self.finality_and_apply_block(
                        &finality_block_info.block,
                        &finality_block_info.block_data.unwrap(),
                        None,
                        false,
                    );

                    if res.is_ok() {
                        info!(target: "node", "incoming block seq_no = {} applied", block_seq_no);
                    } else {
                        warn!(target: "node", "temporary block seq_no = {} apply error {:?}!",
                            block_seq_no, res
                        );
                    }
                }
            } else {
                break;
            }
        }
    }

    /// finality and apply block
    fn finality_and_apply_block(
        &self,
        block: &SignedBlock,
        block_data: &[u8],
        applied_shard: Option<ShardStateUnsplit>,
        is_sync: bool,
    ) -> NodeResult<(Arc<ShardStateUnsplit>, Vec<u128>)> {

        let mut time = Vec::new();
        let now = Instant::now();

        let finality_hash = Vec::new();
        let res = self.block_applier.lock().apply(
            block,
            Some(block_data.to_vec()),
            finality_hash,
            applied_shard,
            is_sync,
        );
        time.push(now.elapsed().as_micros());
        Ok((res?, time))
    }
}
