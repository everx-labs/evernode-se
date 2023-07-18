use crate::engine::engine::TonNodeEngine;
use crate::engine::BlockTimeMode;
use crate::tests::abi_account::GiverV3;
use crate::tests::{blockchain_config, get_config_params, init_logger, mem_storage, DocsReader};
use crate::MemDocumentsDb;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use ton_block::ShardIdent;

const CFG_TEST: &str = include_str!("cfg_test");

#[test]
fn test_node_time_control() {
    init_logger();
    let (_config, _public_keys) = get_config_params(CFG_TEST);
    //    let (_node_index, _port, _private_key, public_keys, _boot_list ) = get_config_params::<NodeConfig>(&json);
    let db = Arc::new(MemDocumentsDb::new());
    let storage = Arc::new(mem_storage());
    let node = Arc::new(
        TonNodeEngine::with_params(
            0,
            ShardIdent::full(0),
            Arc::new(blockchain_config()),
            db.clone(),
            storage,
        )
        .unwrap(),
    );
    node.clone().start().unwrap();
    let mut db_reader = DocsReader::new(db.clone());
    db_reader.dump_next();

    let giver = GiverV3::new();

    node.process_message(giver.send_transaction(
        node.get_next_time(),
        &giver.account.address.to_string(),
        1000000000,
        false,
    ))
    .unwrap();

    let blk = map_vec::<Blk>(db_reader.next_blocks());
    let trx = map_vec::<Trx>(db_reader.next_transactions());
    assert_eq!(blk.len(), 2);
    assert_eq!(blk[1].gen_utime, blk[0].gen_utime);
    assert_eq!(blk[0].workchain_id, 0);
    assert_eq!(blk[1].workchain_id, -1);
    assert_eq!(trx.len(), 2);
    assert_eq!(trx[1].now, trx[0].now);
    assert_eq!(trx[0].now, blk[0].gen_utime);

    node.set_time_mode(BlockTimeMode::Seq);

    node.process_message(giver.send_transaction(
        node.get_next_time(),
        &giver.account.address.to_string(),
        1000000000,
        false,
    ))
    .unwrap();

    let blk = map_vec::<Blk>(db_reader.next_blocks());
    let trx = map_vec::<Trx>(db_reader.next_transactions());
    assert_eq!(blk.len(), 3);
    assert_eq!(blk[1].gen_utime, blk[0].gen_utime + 1);
    assert_eq!(blk[2].gen_utime, blk[1].gen_utime + 1);
    assert_eq!(blk[0].workchain_id, 0);
    assert_eq!(blk[1].workchain_id, 0);
    assert_eq!(blk[2].workchain_id, -1);
    assert_eq!(trx.len(), 2);
    assert_eq!(trx[1].now, trx[0].now + 1);
    assert_eq!(trx[0].now, blk[0].gen_utime);
    assert_eq!(trx[1].now, blk[1].gen_utime);
}

fn map_vec<T: DeserializeOwned>(v: Vec<Value>) -> Vec<T> {
    v.into_iter()
        .map(|x| serde_json::from_value::<T>(x).unwrap())
        .collect::<Vec<T>>()
}

#[derive(Deserialize)]
struct Blk {
    pub gen_utime: u32,
    pub workchain_id: i32,
}

#[derive(Deserialize)]
struct Trx {
    pub now: u32,
}
