use crate::data::DocumentsDbMock;
use crate::engine::engine::{get_config_params, TonNodeEngine};
use std::path::PathBuf;
use std::sync::Arc;
use ton_block::ShardIdent;
use ton_executor::BlockchainConfig;

const CFG_TEST: &str = include_str!("cfg_test");

#[test]
fn test_node_time_control() {
    let (_config, _public_keys) = get_config_params(CFG_TEST);
    //    let (_node_index, _port, _private_key, public_keys, _boot_list ) = get_config_params::<NodeConfig>(&json);
    let db = Arc::new(DocumentsDbMock);
    let node = TonNodeEngine::with_params(
        0,
        ShardIdent::full(0),
        Arc::new(BlockchainConfig::default()),
        db.clone(),
        PathBuf::from("../target"),
    )
    .unwrap();
    node.set_seq_mode(true);
    // node.message_queue.queue().unwrap();
    node.process_messages(true).unwrap();
}
