use crate::tests::parse_address;
use ed25519_dalek::{Keypair, PublicKey, SecretKey};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use ton_abi::token::Tokenizer;
use ton_abi::Contract;
use ton_block::{ExternalInboundMessageHeader, Message, MsgAddressExt, MsgAddressInt};
use ton_types::SliceData;

pub struct AbiAccount {
    pub(crate) address: MsgAddressInt,
    contract: Contract,
    keys: Keypair,
}

impl AbiAccount {
    pub fn new(abi_json: &str, keys_json: &str, address: Option<&str>) -> Self {
        let contract = Contract::load(Cursor::new(abi_json)).unwrap();
        let keys = parse_keypair(keys_json);
        let address = if let Some(address) = address {
            parse_address(address).unwrap()
        } else {
            panic!("Address not specified")
        };
        Self {
            address,
            contract,
            keys,
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

pub struct GiverV3 {
    pub account: Arc<AbiAccount>,
}

impl GiverV3 {
    const ADDRESS: &'static str =
        "0:96137b99dcd65afce5a54a48dac83c0fd276432abbe3ba7f1bfb0fb795e69025";
    const ABI_JSON: &'static str = include_str!("../../../contracts/giver_v3/GiverV3.abi.json");
    const KEYS_JSON: &'static str = include_str!("../../../contracts/giver_v3/seGiver.keys.json");

    pub fn new() -> Self {
        Self {
            account: Arc::new(AbiAccount::new(
                Self::ABI_JSON,
                Self::KEYS_JSON,
                Some(Self::ADDRESS),
            )),
        }
    }

    pub fn send_transaction(&self, time: u32, dest: &str, value: u128, bounce: bool) -> Message {
        self.account.encode_ext_in_msg(
            time,
            "sendTransaction",
            json!({
                "dest": dest,
                "value": value.to_string(),
                "bounce": bounce

            }),
        )
    }
}
