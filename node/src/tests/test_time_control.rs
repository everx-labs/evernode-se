use crate::engine::engine::TonNodeEngine;
use crate::engine::BlockTimeMode;
use crate::error::{NodeError, NodeResult};
use crate::tests::{blockchain_config, get_config_params, mem_storage};
use crate::MemDocumentsDb;
use ed25519_dalek::{Keypair, PublicKey, SecretKey};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;
use ton_abi::token::Tokenizer;
use ton_abi::Contract;
use ton_block::{
    ExternalInboundMessageHeader, GetRepresentationHash, Message, MsgAddressExt, MsgAddressInt,
    ShardIdent,
};
use ton_types::SliceData;

const CFG_TEST: &str = include_str!("cfg_test");

const GIVER_V3_ADDRESS: &str = "0:96137b99dcd65afce5a54a48dac83c0fd276432abbe3ba7f1bfb0fb795e69025";
const GIVER_V3_ABI_JSON: &str = include_str!("../../../contracts/giver_v3/GiverV3.abi.json");
const GIVER_V3_KEYS_JSON: &str = include_str!("../../../contracts/giver_v3/seGiver.keys.json");

#[test]
fn test_node_time_control() {
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
    sleep(Duration::from_secs(5));
    dump_documents_db(&db);

    let giver = Acc::new(GIVER_V3_ADDRESS, GIVER_V3_ABI_JSON, GIVER_V3_KEYS_JSON);
    let send_msg = giver.process_ext_in_msg(
        &node,
        "sendTransaction",
        json!({
            "dest": GIVER_V3_ADDRESS,
            "value": "1000000000",
            "bounce": false

        }),
    );

    println!("{}", send_msg.hash().unwrap().to_hex_string());
    dump_documents_db(&db);

    // node.time.write().set_mode(BlockTimeMode::Seq);
}

fn dump_documents_db(db: &MemDocumentsDb) {
    println!("blocks:");
    for block in db.blocks() {
        println!(
            "{}, {}, {}, {}",
            block["gen_utime"], block["workchain_id"], block["seq_no"], block["id"]
        );
    }
    println!("transactions:");
    for tr in db.transactions() {
        println!(
            "{}, {}, {}, {}, {}, {}, {}",
            tr["now"],
            tr["account_addr"],
            tr["lt"],
            tr["id"],
            tr["aborted"],
            tr["in_msg"],
            tr["out_msgs"]
        );
    }

}

struct Acc {
    address: MsgAddressInt,
    contract: Contract,
    keys: Keypair,
}

impl Acc {
    pub fn new(address: &str, abi_json: &str, keys_json: &str) -> Self {
        Self {
            address: parse_address(address).unwrap(),
            contract: Contract::load(std::io::Cursor::new(abi_json)).unwrap(),
            keys: parse_keypair(keys_json),
        }
    }

    pub fn encode_ext_in_msg(&self, time: u32, function: &str, params: Value) -> Message {
        let func = self.contract.function(function).unwrap();
        let body_builder = func
            .encode_input(
                &Tokenizer::tokenize_optional_params(
                    func.header_params(),
                    &json!({
                        "time": (time as u64) * 1000,
                        "expire": time + 1,
                    }),
                    &HashMap::default(),
                )
                .unwrap(),
                &Tokenizer::tokenize_all_params(func.input_params(), &params).unwrap(),
                false,
                Some(&self.keys),
                Some(self.address.clone()),
            )
            .unwrap();
        let body = SliceData::load_builder(body_builder).unwrap();
        Message::with_ext_in_header_and_body(
            ExternalInboundMessageHeader::new(MsgAddressExt::AddrNone, self.address.clone()),
            body,
        )
    }

    pub fn process_ext_in_msg(&self, node: &TonNodeEngine, function: &str, params: Value) -> Message {
        let msg = self.encode_ext_in_msg(node.get_next_time(), function, params);
        node.message_queue.queue(msg.clone()).unwrap();
        sleep(Duration::from_secs(5));
        msg
    }
}

fn parse_address(string: &str) -> NodeResult<MsgAddressInt> {
    match MsgAddressInt::from_str(string) {
        Ok(address) => Ok(address),
        Err(err) => Err(NodeError::InvalidData(format!(
            "Invalid address [{}]: {}",
            string, err
        ))),
    }
}

fn parse_keypair(json_str: &str) -> Keypair {
    let value = serde_json::from_str::<Value>(json_str).unwrap();
    let public = hex::decode(value["public"].as_str().unwrap()).unwrap();
    let secret = hex::decode(value["secret"].as_str().unwrap()).unwrap();
    Keypair {
        public: PublicKey::from_bytes(&public).unwrap(),
        secret: SecretKey::from_bytes(&secret).unwrap(),
    }
}
