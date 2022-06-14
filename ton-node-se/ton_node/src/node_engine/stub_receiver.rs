use crate::error::NodeResult;
use node_engine::logical_time_generator::LogicalTimeGenerator;
use node_engine::{
    InMessagesQueue, MessagesReceiver, QueuedMessage, ACCOUNTS, ACCOUNTS_COUNT,
    DEPRECATED_GIVER_ABI2_DEPLOY_MSG, GIVER_ABI1_DEPLOY_MSG, GIVER_ABI2_DEPLOY_MSG, GIVER_BALANCE,
    MULTISIG_BALANCE, MULTISIG_DEPLOY_MSG, SUPER_ACCOUNT_ID,
};
use std::sync::{mpsc, Arc};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use ton_block::{
    CommonMsgInfo, CurrencyCollection, Deserializable, ExternalInboundMessageHeader, Grams,
    InternalMessageHeader, Message, MsgAddressExt, MsgAddressInt, Serializable, StateInit,
};
use ton_labs_assembler::compile_code;
use ton_types::{AccountId, Cell, SliceData};

pub struct StubReceiver {
    stop_tx: Option<mpsc::Sender<bool>>,
    join_handle: Option<JoinHandle<()>>,
    workchain_id: i8,
    block_seqno: u32,
    timeout: u64,
}

impl MessagesReceiver for StubReceiver {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {
        if self.block_seqno == 1 {
            Self::deploy_contracts(self.workchain_id, &queue)?;
        }

        if self.timeout == 0 {
            return Ok(());
        }

        for acc in 0..ACCOUNTS_COUNT {
            ACCOUNTS.lock().push(AccountId::from([acc + 1; 32]));
        }

        if self.join_handle.is_none() {
            let (tx, rx) = mpsc::channel();
            self.stop_tx = Some(tx);
            let mut log_time_gen = LogicalTimeGenerator::with_init_value(0);

            let workchain_id = self.workchain_id;
            let timeout = self.timeout;
            self.join_handle = Some(thread::spawn(move || {
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
                queue
                    .queue(
                        QueuedMessage::with_message(Self::create_transfer_message(
                            workchain_id,
                            SUPER_ACCOUNT_ID.clone(),
                            SUPER_ACCOUNT_ID.clone(),
                            1_000_000,
                            log_time_gen.get_current_time(),
                        ))
                        .unwrap(),
                    )
                    .unwrap();
                queue
                    .queue(
                        QueuedMessage::with_message(Self::create_account_message_with_code(
                            workchain_id,
                            SUPER_ACCOUNT_ID.clone(),
                        ))
                        .unwrap(),
                    )
                    .unwrap();
            }));
        }
        Ok(())
    }
}

#[allow(dead_code)]
impl StubReceiver {
    pub fn with_params(workchain_id: i8, block_seqno: u32, timeout: u64) -> Self {
        StubReceiver {
            stop_tx: None,
            join_handle: None,
            workchain_id,
            block_seqno,
            timeout,
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
        Self::create_account_message(
            workchain_id,
            account_id,
            code,
            SliceData::new_empty().into_cell(),
            None,
        )
    }

    fn deploy_contracts(workchain_id: i8, queue: &InMessagesQueue) -> NodeResult<()> {
        Self::deploy_contract(workchain_id, GIVER_ABI1_DEPLOY_MSG, GIVER_BALANCE, 1, queue)?;
        Self::deploy_contract(workchain_id, GIVER_ABI2_DEPLOY_MSG, GIVER_BALANCE, 3, queue)?;
        Self::deploy_contract(
            workchain_id,
            MULTISIG_DEPLOY_MSG,
            MULTISIG_BALANCE,
            5,
            queue,
        )?;
        Self::deploy_contract(
            workchain_id,
            DEPRECATED_GIVER_ABI2_DEPLOY_MSG,
            GIVER_BALANCE,
            7,
            queue,
        )?;

        Ok(())
    }

    fn deploy_contract(
        workchain_id: i8,
        deploy_msg_boc: &[u8],
        initial_balance: u128,
        transfer_lt: u64,
        queue: &InMessagesQueue,
    ) -> NodeResult<AccountId> {
        let (deploy_msg, deploy_addr) =
            Self::create_contract_deploy_message(workchain_id, deploy_msg_boc);
        let transfer_msg = Self::create_transfer_message(
            workchain_id,
            deploy_addr.clone(),
            deploy_addr.clone(),
            initial_balance,
            transfer_lt,
        );
        Self::queue_with_retry(queue, transfer_msg)?;
        Self::queue_with_retry(queue, deploy_msg)?;

        Ok(deploy_addr)
    }

    fn queue_with_retry(queue: &InMessagesQueue, message: Message) -> NodeResult<()> {
        let mut message = QueuedMessage::with_message(message)?;
        while let Err(msg) = queue.queue(message) {
            message = msg;
            thread::sleep(Duration::from_micros(100));
        }

        Ok(())
    }

    fn create_contract_deploy_message(workchain_id: i8, msg_boc: &[u8]) -> (Message, AccountId) {
        let mut msg = Message::construct_from_bytes(msg_boc).unwrap();
        if let CommonMsgInfo::ExtInMsgInfo(ref mut header) = msg.header_mut() {
            match header.dst {
                MsgAddressInt::AddrStd(ref mut addr) => addr.workchain_id = workchain_id,
                _ => panic!("Contract deploy message has invalid destination address"),
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
        body: Option<SliceData>,
    ) -> Message {
        let code_cell = compile_code(code).unwrap().into_cell();

        let mut msg = Message::with_ext_in_header(ExternalInboundMessageHeader {
            src: MsgAddressExt::default(),
            dst: MsgAddressInt::with_standart(None, workchain_id, account_id.clone()).unwrap(),
            import_fee: Grams::zero(),
        });

        let mut state_init = StateInit::default();
        state_init.set_code(code_cell);
        state_init.set_data(data);
        msg.set_state_init(state_init);

        if let Some(body) = body {
            msg.set_body(body);
        }
        msg
    }

    // create transfer funds message for initialize balance
    pub fn create_transfer_message(
        workchain_id: i8,
        src: AccountId,
        dst: AccountId,
        value: u128,
        lt: u64,
    ) -> Message {
        let mut balance = CurrencyCollection::default();
        balance.grams = value.into();

        let mut msg = Message::with_int_header(InternalMessageHeader::with_addresses_and_bounce(
            MsgAddressInt::with_standart(None, workchain_id, src).unwrap(),
            MsgAddressInt::with_standart(None, workchain_id, dst).unwrap(),
            balance,
            false,
        ));

        msg.set_at_and_lt(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs() as u32,
            lt,
        );
        msg
    }

    // Create message "from wallet" to transfer some funds
    // from one account to another
    pub fn create_external_transfer_funds_message(
        workchain_id: i8,
        src: AccountId,
        dst: AccountId,
        value: u128,
        _lt: u64,
    ) -> Message {
        let mut msg = Message::with_ext_in_header(ExternalInboundMessageHeader {
            src: MsgAddressExt::default(),
            dst: MsgAddressInt::with_standart(None, workchain_id, src.clone()).unwrap(),
            import_fee: Grams::zero(),
        });

        msg.set_body(
            Self::create_transfer_int_header(workchain_id, src, dst, value)
                .serialize()
                .unwrap()
                .into(),
        );

        msg
    }

    pub fn create_transfer_int_header(
        workchain_id: i8,
        src: AccountId,
        dest: AccountId,
        value: u128,
    ) -> InternalMessageHeader {
        let msg = Self::create_transfer_message(workchain_id, src, dest, value, 0);
        match msg.withdraw_header() {
            CommonMsgInfo::IntMsgInfo(int_hdr) => int_hdr,
            _ => panic!("must be internal message header"),
        }
    }

    fn try_receive_message(
        workchain_id: i8,
        log_time_gen: &mut LogicalTimeGenerator,
    ) -> Option<Message> {
        let time = log_time_gen.get_next_time();
        Some(match time - 1 {
            x if x < (ACCOUNTS_COUNT as u64) => Self::create_transfer_message(
                workchain_id,
                SUPER_ACCOUNT_ID.clone(),
                ACCOUNTS.lock()[x as usize].clone(),
                1000000,
                log_time_gen.get_current_time(),
            ),
            x if x >= (ACCOUNTS_COUNT as u64) && x < (ACCOUNTS_COUNT as u64) * 2 => {
                Self::create_transfer_message(
                    workchain_id,
                    SUPER_ACCOUNT_ID.clone(),
                    ACCOUNTS.lock()[(x - ACCOUNTS_COUNT as u64) as usize].clone(),
                    1000000,
                    log_time_gen.get_current_time(),
                )
            }
            x if x >= (ACCOUNTS_COUNT as u64) * 2 && x < (ACCOUNTS_COUNT as u64) * 3 => {
                let index = (x - (ACCOUNTS_COUNT as u64) * 2) as usize;
                Self::create_account_message_with_code(workchain_id, ACCOUNTS.lock()[index].clone())
            }
            x => {
                // send funds from 1 to 2, after from 2 to 3 and etc
                let acc_src = (x % ACCOUNTS_COUNT as u64) as usize;
                let acc_dst = (acc_src + 1) % ACCOUNTS_COUNT as usize;
                let src_acc_id = ACCOUNTS.lock()[acc_src].clone();
                let dst_acc_id = ACCOUNTS.lock()[acc_dst].clone();
                Self::create_external_transfer_funds_message(
                    workchain_id,
                    src_acc_id,
                    dst_acc_id,
                    rand::random::<u8>() as u128,
                    log_time_gen.get_current_time(),
                )
            }
        })
    }
}
