use super::*;
use adnl::error::{AdnlError, AdnlErrorKind, AdnlResult};
use adnl::server::AdnlServerHandler;
use sha2::{Digest, Sha256};
use std::boxed::Box;
use adnl::common::{serialize, deserialize_variant};
use std::{io::Cursor, time::{ SystemTime, UNIX_EPOCH }};
use ton_api::IntoBoxed;
use ton_api::ton::{int256, lite_server, rpc, TLObject, ton_node};
use ton_api::ton::adnl::message::message::Query;
use ton_block::{Deserializable, Message, MerkleProof};
use ton_types::cells_serialization::{BagOfCells, serialize_toc};
use std::convert::TryInto;

impl AdnlServerHandler for TonNodeEngine {
    fn process_query(&self, q: &Box<Query>) -> AdnlResult<Vec<u8>> {
        let obj = deserialize_variant(&q.query[..])?;
        match obj.downcast::<rpc::lite_server::Query>() {
            Ok(q) => {
                let obj = deserialize_variant(&q.data[..])?;
                if obj.is::<rpc::lite_server::GetAccountState>() {
                    return self.process_get_account_state(obj);
                }      
                if obj.is::<rpc::lite_server::GetBlock>() {
                    return self.process_get_block_data(obj);
                }
                if obj.is::<rpc::lite_server::GetMasterchainInfo>() {
                    return self.process_get_masterchain_info();
                }
                if obj.is::<rpc::lite_server::GetTime>() {
                    return self.process_get_time();
                }
                if obj.is::<rpc::lite_server::SendMessage>() {
                    return self.process_send_message(obj);
                }
            },
            _ => ()
        }
        Err(adnl_err!(AdnlErrorKind::UnsupportedQuery(q.clone())))
    }
}

impl TonNodeEngine {
    fn process_get_masterchain_info(&self) -> AdnlResult<Vec<u8>> {
        info!(target: "adnl", "process_get_masterchain_info");
        let last_block_info = self.finalizer
            .lock()
            .get_last_block_info()
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
            )?;
        let shard = self.current_shard_id();
        serialize(
            &lite_server::masterchaininfo::MasterchainInfo { 
                last: ton_node::blockidext::BlockIdExt {
                    workchain: shard.workchain_id(),
                    shard: shard.shard_prefix_with_tag() as i64,
                    seqno: last_block_info.prev1()?.seq_no as i32,
                    root_hash: int256(last_block_info.prev1()?.root_hash.as_slice().clone()),
                    file_hash: int256(last_block_info.prev1()?.file_hash.as_slice().clone())
                },
                state_root_hash: int256([0; 32]),
                init: ton_node::zerostateidext::ZeroStateIdExt {
                    workchain: shard.workchain_id(),
                    root_hash: int256([0; 32]), // TODO i don`t know yet what is it
                    file_hash: int256([0; 32])
                }
            }.into_boxed()
        )
    }

    fn process_get_time(&self) -> AdnlResult<Vec<u8>> {
        info!(target: "adnl", "process_get_time");
        serialize(
            &lite_server::currenttime::CurrentTime { 
                now: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i32
            }.into_boxed()
        )
    }

    fn process_get_block_data(&self, block_info: TLObject) -> AdnlResult<Vec<u8>> {

        info!(target: "adnl", "process_get_block_data");
        let block_info = block_info.downcast::<rpc::lite_server::GetBlock>()
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::UnexpectedTlObject(err))
            )?;
        info!(target: "adnl", "get_block_data {:?}", block_info);

        let sign_block_raw = self.finalizer
            .lock()
            .get_raw_block_by_seqno(block_info.id.seqno as u32, 0)
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
            )?;

        let sblock = SignedBlock::read_from(&mut Cursor::new(sign_block_raw))
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
            )?;
        let block = sblock.block();
        let mut block_data = vec![];
        let cell = block.serialize()
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
            )?;

        let bag = BagOfCells::with_root(&cell);
        bag.write_to(&mut block_data, false)
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
            )?;

        let mut hasher = Sha256::new();
        hasher.input(block_data.as_slice());
        let hash: [u8; 32] = hasher.result().to_vec().try_into().unwrap();
        let file_hash = UInt256::from(hash);
        assert_eq!(file_hash, UInt256::from(block_info.id.file_hash.0));

        let shard = self.current_shard_id();
        serialize(
            &lite_server::blockdata::BlockData {
                id: ton_node::blockidext::BlockIdExt {
                    workchain: shard.workchain_id(),
                    shard: shard.shard_prefix_with_tag() as i64,
                    seqno: block.read_info()?.seq_no() as i32,
                    root_hash: int256(block.hash().unwrap().as_slice().clone()),
                    file_hash: int256(file_hash.as_slice().clone())
                },
                data: ton_api::ton::bytes(block_data)
            }.into_boxed()
        )

    }

    fn process_get_account_state(&self, acc_info: TLObject) -> AdnlResult<Vec<u8>> {

        info!(target: "adnl", "process_get_account_state");
        let acc_info = acc_info.downcast::<rpc::lite_server::GetAccountState>()
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::UnexpectedTlObject(err))
            )?;
        info!(
            target: "adnl", 
            "workchain: {}, account address {:?}", 
            acc_info.account.workchain, 
            acc_info.account.id 
        );

        let finalizer = self.finalizer.lock();

        let last_block_info = finalizer
            .get_last_block_info()
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string())))?;
        
        let shard = self.current_shard_id();
        let last_shard_state = finalizer.get_last_shard_state();
        let last_shard_root = last_shard_state.serialize()?;
        let acc = last_shard_state.read_accounts()?
        .account(&AccountId::from(acc_info.account.id.0))?;
        
        if let Some(acc) = acc {
            let account_boc = serialize_toc(&acc.account_cell())?;
            let acc_repr_hash = acc.account_cell().repr_hash();

            // proof from block with full `BlockInfo` and 
            // `MerkleUpdate`'s root (other parts of block - pruned brunches)
            let signed_block_bytes = finalizer
                .get_raw_block_by_seqno(last_block_info.prev1()?.seq_no as u32, 0)
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
                )?;
    
            let signed_block = SignedBlock::read_from(&mut Cursor::new(signed_block_bytes))
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
                )?;

            let block = signed_block.block();
            let block_root = block.serialize()
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
                )?;
            let block_root: Cell = block_root.into();

            let block_info_root = block.info_cell();
            let block_info_cells = BagOfCells::with_root(&block_info_root)
                .withdraw_cells();

            let is_include = |h: &ton_types::UInt256| {
                info!(target: "adnl", "process_get_account_state 200 {}", h);
                h == block_root.reference(2).unwrap().repr_hash() || // Merkle update root
                block_info_cells.contains_key(h)
            };

            let block_proof = MerkleProof::create(&block_root, &is_include)
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(
                        "block proof: ".to_string() + &err.to_string())
                    )
                )?;
            let block_proof_root = block_proof.serialize()
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
                )?;

            // proof for AccountState in ShardStateUnsplit struct
            let second_ref = last_shard_root.reference(2).unwrap().repr_hash();
            let is_include = |h: &ton_types::UInt256| {
                info!(target: "adnl", "process_get_account_state 218 {}", h);
                &h == &second_ref || // part of  ShardStateUnsplit in separate cell ^[ ... ]
                &h == &acc_repr_hash
            };
            let acc_proof = MerkleProof::create(
                &last_shard_root, &is_include)
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(
                        "account proof: ".to_string() + &err.to_string())
                    )
                )?;
            let acc_proof_root = acc_proof.serialize()
                .map_err(|err| 
                    adnl_err!(AdnlErrorKind::QueryProcessingFailed(err.to_string()))
                )?;
            
            // Proof contains 2 roots with 2 proofs
            let proof_bag = BagOfCells::with_roots(
                vec![&block_proof_root.into(), &acc_proof_root]
             );
            
            //println!("*** proof_bag\n{}\n", proof_bag);

            let mut proof_bytes = vec![];
            proof_bag.write_to(&mut proof_bytes, false)?;
            serialize(
                &lite_server::accountstate::AccountState {
                    id: acc_info.id,
                    shardblk: ton_node::blockidext::BlockIdExt {
                        workchain: shard.workchain_id(),
                        shard: shard.shard_prefix_with_tag() as i64,
                        seqno: last_block_info.prev1()?.seq_no as i32,
                        root_hash: int256(last_block_info.prev1()?.root_hash.as_slice().clone()),
                        file_hash: int256(last_block_info.prev1()?.file_hash.as_slice().clone())
                    },
                    shard_proof: ton_api::ton::bytes(vec![]), // is used when account belongs some workchain
                    proof: ton_api::ton::bytes(proof_bytes),
                    state: ton_api::ton::bytes(account_boc),
                }.into_boxed()
            )

        } else {
            warn!(target: "adnl", "process_get_account_state cannot find account {:?}", acc_info.account.id.0);
            Ok(Vec::new())
        }
    }

    fn process_send_message(&self, message: ton_api::ton::TLObject) -> AdnlResult<Vec<u8>> {
        info!(target: "adnl", "process_send_message");
        let message = message.downcast::<rpc::lite_server::SendMessage>()
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::UnexpectedTlObject(err))
            )?;
        let msg = Message::construct_from_bytes(&message.body.0)?;
        let ret = self.message_queue
            .queue(QueuedMessage::with_message(msg).unwrap())
            .map_err(|_| { 
                warn!(target: "adnl", "Node message queue is full");
                adnl_err!(AdnlErrorKind::QueueIsFull)
            });
        serialize(
            &lite_server::sendmsgstatus::SendMsgStatus {
                status: if ret.is_ok() {
                    1
                } else {
                    0
                }
            }.into_boxed()
        )
    }

}
