#[macro_use]
extern crate clap;
extern crate hex;
extern crate ton_block;
extern crate ton_node_old;
extern crate ton_types;
extern crate ton_vm as tvm;

use std::fs::File;
use std::io::Write;
use std::sync::Arc;

use clap::AppSettings;
use serde_json::Value;
use ton_block::Serializable;
use ton_client::abi::{Abi, CallSet, DeploySet, encode_message, FunctionHeader, ParamsOfEncodeMessage, Signer};
use ton_client::ClientContext;
use ton_client::crypto::KeyPair;
use ton_types::{
    AccountId, BagOfCells
};

use ton_node_old::node_engine::StubReceiver as MsgCreator;

#[tokio::main]
async fn main() {
    let args = clap_app!(create_msg =>
        (version: &*format!("{}\nCOMMIT_ID: {}\nBUILD_DATE: {}\nCOMMIT_DATE: {}\nGIT_BRANCH: {}",
            env!("CARGO_PKG_VERSION"),
            env!("BUILD_GIT_COMMIT"),
            env!("BUILD_TIME") ,
            env!("BUILD_GIT_DATE"),
            env!("BUILD_GIT_BRANCH")
        ))
        (author: "TONLabs")
        (about: "Message creator in BoC format")
        (@subcommand transfer =>
            (about: "Transfer funds")
            (@arg SRC: +required +takes_value "Source account address")
            (@arg DST: +required +takes_value "Destination account address")
            (@arg VALUE: +required +takes_value "Value for transfer (nanotokens)")
            (@arg OUTPUT: +required +takes_value "Output file name")
        )
        (@subcommand deploy =>
            (@arg TVC: +required +takes_value "Compiled smart contract (tvc file)")
            (@arg PARAMS: +required +takes_value "Constructor arguments. Can be passed via a filename.")
            (@arg ABI: --abi +required +takes_value "Json file with contract ABI.")
            (@arg WC: --wc +takes_value "Workchain id of the smart contract (default 0).")
            (@arg SIGN: --sign +required +takes_value "Keypair used to sign 'constructor message'.")
            (@arg OUTPUT: +required +takes_value "Output file name")
        )
    )
        .setting(AppSettings::ArgRequiredElseHelp)
        .get_matches();

    let (subcommand, args) = args.subcommand();
    let args = args.expect("Specify SUBCOMMAND: transfer or deploy");
    let out = args.value_of("OUTPUT")
        .expect("Specify OUTPUT file name");
    match subcommand {
        "transfer" => {
            let src = args.value_of("SRC").unwrap();
            let dst = args.value_of("DST").unwrap();
            let value: u128 = args.value_of("VALUE").unwrap().parse().expect("Error parse value");
            assert_eq!(src.len(), 64, "source address is invalid. Address can be only 32 byte hex string");
            assert_eq!(dst.len(), 64, "destination address is invalid. Address can be only 32 byte hex string");
            let source_vec = hex::decode(src)
                .expect("source address is invalid hex string");
            let dest_vec = hex::decode(dst)
                .expect("source address is invalid hex string");

            let message = MsgCreator::create_external_transfer_funds_message(
                0,
                AccountId::from(source_vec),
                AccountId::from(dest_vec),
                value,
                0);

            let b = message.write_to_new_cell().expect("Error write message to tree of cells");
            let bag = BagOfCells::with_root(&b.into());

            let mut file = File::create(out.clone()).expect("Error create out file");
            bag.write_to(&mut file, false).expect("Error write message to file");
            file.flush().expect("Error flush out file");

            println!("BoC successfully saved: {}", out);
        },
        "deploy" => {
            let tvc_file_name = args.value_of("TVC").unwrap();
            let tvc = std::fs::read(tvc_file_name)
                .expect("Error reading TVC file");
            let tvc = base64::encode(tvc);

            let abi_file_name = args.value_of("ABI").unwrap();
            let abi = std::fs::read_to_string(abi_file_name)
                .expect("Error reading ABI file");
            let abi = serde_json::from_str(&abi)
                .expect("ABI file parsing failed");

            let wc = args.value_of("WC")
                .map(|value| value.parse().expect("Workchain id must be valid integer value"))
                .unwrap_or(0);

            let params = serde_json::from_str(args.value_of("PARAMS")
                .expect("Expected params"))
                .expect("PARAMS must be valid JSON");

            let keypair_file_name = args.value_of("SIGN").unwrap();
            let keypair = std::fs::read_to_string(keypair_file_name)
                .expect("Error reading keypair file");

            let keypair: Value = serde_json::from_str(&keypair)
                .expect("Keypair file must be valid JSON file");

            let encoded = encode_message(
                Arc::new(ClientContext::new(Default::default()).unwrap()),
                ParamsOfEncodeMessage {
                    abi: Abi::Contract(abi),
                    deploy_set: Some(DeploySet {
                        tvc,
                        workchain_id: Some(wc),
                        ..Default::default()
                    }),
                    call_set: Some(CallSet {
                        function_name: "constructor".to_string(),
                        header: Some(FunctionHeader {
                            expire: Some(u32::max_value()),
                            ..Default::default()
                        }),
                        input: Some(params),
                        ..Default::default()
                    }),
                    signer: Signer::Keys {
                        keys: KeyPair::new(
                            keypair["public"].as_str().unwrap().to_string(),
                            keypair["secret"].as_str().unwrap().to_string()
                        )
                    },
                    ..Default::default()
                },
            ).await
                .expect("Error encoding message");

            println!("Address for deployment: {}", encoded.address);
            let decoded = base64::decode(&encoded.message)
                .expect("Error deciding BASE64 message");
            std::fs::write(out,decoded)
                .expect("Error writing output file");

            println!("BoC successfully saved: {}", out);
        },
        _ => {
            println!("Invalid type option");
        }
    }
}
