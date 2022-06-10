use parking_lot::{Condvar, Mutex};
use std::mem;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use ton_block::*;
use ton_types::Cell;
use ton_types::HashmapType;
use ton_types::Result;

#[cfg(test)]
#[path = "../../../tonos-se-tests/unit/test_block_builder.rs"]
mod tests;

#[derive(Debug)]
pub struct AppendSerializedContext {
    pub in_msg: Cell,
    pub out_msgs: Vec<(Cell, CurrencyCollection)>,
    pub transaction: Arc<Transaction>,
    pub transaction_cell: Cell,
    pub max_lt: u64,
    pub imported_value: Option<CurrencyCollection>,
    pub exported_value: CurrencyCollection,
    pub imported_fees: ImportFees,
    pub exported_fees: Grams,
}

#[derive(Debug)]
enum BuilderIn {
    Append {
        in_msg: Arc<InMsg>,
        out_msgs: Vec<OutMsg>,
    },
    AppendSerialized {
        value: AppendSerializedContext,
    },
}

///
/// Data for constructing block
///
struct BlockData {
    block_info: BlockInfo,
    block_extra: BlockExtra,
    value_flow: ValueFlow,
    //log_time_gen: LogicalTimeGenerator,
    end_lt: u64, // biggest logical time of all messages
    p1: Duration,
    p2: Duration,
    p3: Duration,
}

impl BlockData {
    fn insert_serialized_in_msg(&mut self, msg_cell: &Cell, fees: &ImportFees) -> Result<()> {
        let mut in_msg_descr = self.block_extra.read_in_msg_descr()?;
        in_msg_descr.insert_serialized(
            &msg_cell.repr_hash().serialize()?.into(),
            &msg_cell.into(),
            fees,
        )?;
        self.block_extra.write_in_msg_descr(&in_msg_descr)
    }

    fn insert_serialized_out_msg(
        &mut self,
        msg_cell: &Cell,
        exported: &CurrencyCollection,
    ) -> Result<()> {
        let mut out_msg_descr = self.block_extra.read_out_msg_descr()?;
        out_msg_descr.insert_serialized(
            &msg_cell.repr_hash().serialize()?.into(),
            &msg_cell.into(),
            exported,
        )?;
        self.block_extra.write_out_msg_descr(&out_msg_descr)
    }
}

///
/// BlockBuilder structure
///
pub struct BlockBuilder {
    sender: Mutex<Option<Sender<BuilderIn>>>,
    current_block_data: Arc<Mutex<BlockData>>,
    stop_event: Arc<Condvar>,
    block_gen_utime: UnixTime32,
    start_lt: u64,
}

impl Default for BlockBuilder {
    fn default() -> Self {
        BlockBuilder::with_params(1, BlkPrevInfo::default(), 0, None)
    }
}

impl BlockBuilder {
    ///
    /// Initialize BlockBuilder
    ///
    pub fn with_params(
        seq_no: u32,
        prev_ref: BlkPrevInfo,
        vert_sec_no: u32,
        prev_vert_ref: Option<BlkPrevInfo>,
    ) -> Self {
        let end_lt = prev_ref.prev1().unwrap().end_lt;
        let mut block_info = BlockInfo::default();
        block_info.set_seq_no(seq_no).unwrap();
        block_info.set_prev_stuff(false, &prev_ref).unwrap();
        block_info
            .set_vertical_stuff(0, vert_sec_no, prev_vert_ref)
            .unwrap();
        block_info.set_gen_utime(UnixTime32::now());
        block_info.set_start_lt(end_lt + 1);

        Self::init_with_block_info(block_info)
    }

    ///
    /// Initialize BlockBuilder with shard identifier
    ///
    pub fn with_shard_ident(
        shard_ident: ShardIdent,
        seq_no: u32,
        prev_ref: BlkPrevInfo,
        vert_sec_no: u32,
        prev_vert_ref: Option<BlkPrevInfo>,
        block_at: u32,
    ) -> Self {
        let end_lt = prev_ref.prev1().unwrap().end_lt;
        let mut block_info = BlockInfo::default();
        block_info.set_shard(shard_ident);
        block_info.set_seq_no(seq_no).unwrap();
        block_info.set_prev_stuff(false, &prev_ref).unwrap();
        block_info
            .set_vertical_stuff(0, vert_sec_no, prev_vert_ref)
            .unwrap();
        block_info.set_gen_utime(UnixTime32(block_at));
        block_info.set_start_lt(end_lt + 1);

        Self::init_with_block_info(block_info)
    }

    fn init_with_block_info(block_info: BlockInfo) -> Self {
        // channel for inbound messages
        let (sender, receiver) = channel::<BuilderIn>();

        let block_gen_utime = block_info.gen_utime().clone();
        let start_lt = block_info.start_lt();

        let stop_event = Arc::new(Condvar::new());
        let current_block_data = Arc::new(Mutex::new(BlockData {
            block_info,
            block_extra: BlockExtra::default(),
            value_flow: ValueFlow::default(),
            end_lt: 0,
            p1: Duration::new(0, 0),
            p2: Duration::new(0, 0),
            p3: Duration::new(0, 0),
        }));

        let block = BlockBuilder {
            sender: Mutex::new(Some(sender)),
            stop_event: stop_event.clone(),
            current_block_data: current_block_data.clone(),
            block_gen_utime,
            start_lt,
        };

        thread::spawn(move || {
            Self::thread(receiver, current_block_data, stop_event);
        });

        block
    }

    ///
    /// Thread for processing transaction for a block
    ///
    fn thread(
        receiver: Receiver<BuilderIn>,
        current_block_data: Arc<Mutex<BlockData>>,
        stop_event: Arc<Condvar>,
    ) {
        let n = Instant::now();
        for msg in receiver {
            let now = Instant::now();
            match msg {
                BuilderIn::Append { in_msg, out_msgs } => {
                    let mut block_data = current_block_data.lock();
                    if let Err(err) = Self::append_messages(&mut block_data, &in_msg, out_msgs) {
                        log::error!("error append messages {}", err);
                    }
                }
                BuilderIn::AppendSerialized { value } => {
                    if let Err(err) =
                        Self::append_serialized_messages(current_block_data.clone(), value)
                    {
                        log::error!("error append transaction {}", err);
                    }
                }
            }
            let d = now.elapsed();
            current_block_data.lock().p2 += d;
        }
        let d = n.elapsed();
        current_block_data.lock().p3 = d;

        let mut block_data = current_block_data.lock();
        // set end logical time - logical time of last message of last transaction
        let end_lt = block_data.end_lt;
        block_data.block_info.set_end_lt(end_lt);

        stop_event.notify_one();
    }

    pub fn append_to_block(&self, in_msg: InMsg, out_msgs: Vec<OutMsg>) -> Result<()> {
        let mut block_data = self.current_block_data.lock();
        Self::append_messages(&mut block_data, &in_msg, out_msgs)
    }

    fn append_messages(
        block_data: &mut BlockData,
        in_msg: &InMsg,
        mut out_msgs: Vec<OutMsg>,
    ) -> Result<()> {
        if let Some(ref transaction) = in_msg.read_transaction()? {
            // save last lt
            if block_data.end_lt < transaction.logical_time() {
                block_data.end_lt = transaction.logical_time();
            }

            let mut account_blocks = block_data.block_extra.read_account_blocks()?;
            account_blocks.add_transaction(transaction)?;
            block_data
                .block_extra
                .write_account_blocks(&account_blocks)?;
        }

        // calculate ValueFlow
        // imported increase to in-value
        let fee = in_msg.get_fee()?;
        block_data.value_flow.imported.add(&fee.value_imported)?;

        // exported increase to out values
        let mut out_value = CurrencyCollection::new();
        let mut fees_collected = Grams::zero();
        //let mut first_msg = true;
        let now = Instant::now();
        for out_msg in out_msgs.iter() {
            let out_msg_val = out_msg.exported_value()?;
            out_value.add(&out_msg_val)?;
            out_value.grams.add(&out_msg_val.grams)?;
            fees_collected.add(&out_msg_val.grams)?;

            // save last lt
            if let Some((_msg_at, msg_lt)) = out_msg.at_and_lt()? {
                if block_data.end_lt < msg_lt {
                    block_data.end_lt = msg_lt;
                }
            }
        }
        block_data.p1 += now.elapsed();
        block_data.value_flow.exported.add(&out_value)?;

        // TODO need to understand how to calculate fees...
        block_data
            .value_flow
            .fees_collected
            .grams
            .add(&fees_collected)?;

        // next only for masterchain
        // block_data.value_flow.fees_imported =
        // block_data.value_flow.created =
        // block_data.value_flow.minted =

        // append in_msg to in_msg_descr
        let now = Instant::now();
        let mut in_msg_descr = block_data.block_extra.read_in_msg_descr()?;
        in_msg_descr.insert(&in_msg)?;
        block_data.block_extra.write_in_msg_descr(&in_msg_descr)?;
        block_data.p2 += now.elapsed();

        // append out_msgs to out_msg_descr
        let now = Instant::now();
        let mut out_msg_descr = block_data.block_extra.read_out_msg_descr()?;
        for msg in out_msgs.drain(..) {
            out_msg_descr.insert(&msg)?;
        }
        block_data.block_extra.write_out_msg_descr(&out_msg_descr)?;
        block_data.p3 += now.elapsed();
        Ok(())
    }

    fn append_serialized_messages(
        block_data: Arc<Mutex<BlockData>>,
        mut value: AppendSerializedContext,
    ) -> Result<()> {
        debug!(
            "Inserting transaction {}",
            value.transaction_cell.repr_hash().to_hex_string()
        );

        let now = Instant::now();
        let mut block_data = block_data.lock();

        let mut account_blocks = block_data.block_extra.read_account_blocks()?;
        account_blocks.add_serialized_transaction(&value.transaction, &value.transaction_cell)?;
        block_data
            .block_extra
            .write_account_blocks(&account_blocks)?;

        // calculate ValueFlow
        // imported increase to in-value
        if let Some(in_value) = value.imported_value {
            block_data.value_flow.imported.add(&in_value)?;
        }

        // exported increase to out values
        block_data.value_flow.exported.add(&value.exported_value)?;

        // TODO need to understand how to calculate fees...
        block_data
            .value_flow
            .fees_collected
            .grams
            .add(&value.exported_fees)?;

        // next only for masterchain
        // TODO need to do in the future
        // block_data.value_flow.fees_imported =
        // block_data.value_flow.created =
        // block_data.value_flow.minted =

        // append in_msg to in_msg_descr
        block_data.insert_serialized_in_msg(&value.in_msg, &value.imported_fees)?;

        // append out_msgs to out_msg_descr
        for (msg_cell, exported) in value.out_msgs.drain(..) {
            block_data.insert_serialized_out_msg(&msg_cell, &exported)?;
        }

        if block_data.end_lt < value.max_lt {
            block_data.end_lt = value.max_lt;
        }
        let d = now.elapsed();
        block_data.p1 += d;
        debug!(
            "Transaction inserted {}",
            value.transaction_cell.repr_hash().to_hex_string()
        );
        Ok(())
    }

    ///
    /// Add transaction to block
    ///
    pub fn add_transaction(&self, in_msg: Arc<InMsg>, out_msgs: Vec<OutMsg>) -> bool {
        if let Some(sender) = self.sender.lock().as_ref() {
            sender.send(BuilderIn::Append { in_msg, out_msgs }).is_ok()
        } else {
            false
        }
    }

    ///
    /// Add serialized transaction to block
    ///
    pub fn add_serialized_transaction(&self, value: AppendSerializedContext) -> bool {
        if let Some(sender) = self.sender.lock().as_ref() {
            sender.send(BuilderIn::AppendSerialized { value }).is_ok()
        } else {
            false
        }
    }

    ///
    /// Stop processing messages thread.
    ///
    fn brake_block_builder_thread(&self) {
        let mut sender = self.sender.lock();
        *sender = None;
    }

    ///
    /// Check if BlockBuilder is Empty
    ///
    pub fn is_empty(&self) -> bool {
        let block_data = self.current_block_data.lock();
        block_data
            .block_extra
            .read_in_msg_descr()
            .unwrap()
            .is_empty()
            && block_data
                .block_extra
                .read_out_msg_descr()
                .unwrap()
                .is_empty()
            && block_data
                .block_extra
                .read_account_blocks()
                .unwrap()
                .is_empty()
    }

    ///
    /// Get UNIX time and Logical Time of current block
    ///
    pub fn at_and_lt(&self) -> (u32, u64) {
        (self.block_gen_utime.0, self.start_lt)
    }
    pub fn end_lt(&self) -> u64 {
        self.current_block_data.lock().end_lt
    }

    ///
    /// Complete the construction of the block and return it.
    /// returns generated block and new shard state bag (and transaction count)
    ///
    pub fn finalize_block(
        &self,
        shard_state: &ShardStateUnsplit,
        new_shard_state: &ShardStateUnsplit,
    ) -> Result<(Block, usize)> {
        let mut time = [0u128; 3];
        let now = Instant::now();

        let mut block_data = self.current_block_data.lock();

        self.brake_block_builder_thread();
        self.stop_event.wait(&mut block_data);

        time[0] = now.elapsed().as_micros();
        let now = Instant::now();

        // merkle updates for account_blocks calculating
        let mut account_blocks = vec![];
        let mut account_blocks_tree = block_data.block_extra.read_account_blocks()?;
        account_blocks_tree.iterate_objects(|mut account_block| {
            match account_block.calculate_and_write_state(shard_state, new_shard_state) {
                Ok(_) => account_blocks.push(account_block),
                Err(err) => warn!(target: "node", "Error update account state {}", err.to_string()),
            }
            Ok(true)
        })?;

        let mut tr_count = 0;
        for account_block in account_blocks.drain(..) {
            tr_count += account_block.transaction_count().unwrap();
            account_blocks_tree.insert(&account_block)?;
        }
        block_data
            .block_extra
            .write_account_blocks(&account_blocks_tree)?;
        let new_ss_root = new_shard_state.serialize()?;
        let old_ss_root = shard_state.serialize()?;
        /*
                let mut filename = std::path::PathBuf::new();
                let str1 = format!("./shard_state_{}", block_data.block_info.seq_no);
        info!(target: "ton_block", "want to save shard state {}", str1);
                filename.push(str1);
                let mut file_info = std::fs::File::create(filename)?;
                new_bag.write_to(&mut file_info, false)?;
                file_info.flush()?;
                for i in 500..510 {
                    if block_data.block_info.seq_no < i {
                        break;
                    }
                    let str2 = format!("./shard_state_{}", block_data.block_info.seq_no - i);
        info!(target: "ton_block", "want to remove shard state {}", str2);
                    let _ = std::fs::remove_file(str2);
                }
        */

        let state_update = MerkleUpdate::create(&old_ss_root, &new_ss_root)?;

        time[1] = now.elapsed().as_micros();
        let now = Instant::now();

        let block = Block::with_params(
            0,
            mem::take(&mut block_data.block_info),
            mem::take(&mut block_data.value_flow),
            state_update,
            mem::take(&mut block_data.block_extra),
        )?;

        time[2] = now.elapsed().as_micros();

        info!(target: "profiler",
            "Block builder time: {} / {} / {}",
            time[0], time[1], time[2]
        );
        info!(target: "profiler",
            "Block builder thread time: {} / {} / {}",
            block_data.p1.as_micros(),
            block_data.p2.as_micros(),
            block_data.p3.as_micros(),
        );

        Ok((block, tr_count))
    }
}
