extern crate clap;
extern crate hex;
extern crate ton_vm as tvm;
extern crate ton_block;
extern crate ton_types;
extern crate ton_node_old;

use clap::{App, Arg};
use std::fs::{File};
use std::io::{Write, Read};
use ton_block::Serializable;
use ton_types::{
    AccountId, BagOfCells, BuilderData
};
use ton_node_old::node_engine::StubReceiver as MsgCreator;

fn main() {
    run().expect("Error run node");
}

fn run() -> Result<(), ()> {
    
    println!("Message creator in BoC format v{}\nCOMMIT_ID: {}\nBUILD_DATE: {}\nCOMMIT_DATE: {}\nGIT_BRANCH: {}",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_GIT_COMMIT"),
        env!("BUILD_TIME") ,
        env!("BUILD_GIT_DATE"),
        env!("BUILD_GIT_BRANCH"));


    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .arg(
            Arg::with_name("type")
                .help("Type od message: transafer funds (transfer), deploy account code and data (deploy)")
                .long("type")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("src")
                .help("sourc
                e account address")
                .long("src")
                .required(false)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("dst")
                .help("destination account address")
                .long("dst")
                .required(false)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("value")
                .help("value for transfer (nanograms)")
                .long("value")
                .required(false)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("code")
                .help("file name with code of smart-contract")
                .long("code")
                .required(false)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("data")
                .help("file name with persistent data")
                .long("data")
                .required(false)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("body")
                .help("file name with message body")
                .long("body")
                .required(false)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("out")
                .help("output file name for storage message")
                .long("out")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .get_matches();

    let t: String = args.value_of("type").unwrap().parse().expect("Error parse type");
    let out: String = args.value_of("out").unwrap().parse().expect("Error parse out");
    match t.as_ref() {
        "transfer" => {
            if !args.is_present("src") || !args.is_present("dst") || !args.is_present("value") {
                println!("transfer usage:");
                println!("--type transfer --src <src acc id> --dst <dst acc id> --value <nanograms> --out <output message> ");
                return Err(());
            }

            let src: String = args.value_of("src").unwrap().parse().expect("Error parse source address");
            let dst: String = args.value_of("dst").unwrap().parse().expect("Error parse destination address");
            let value: u128 = args.value_of("value").unwrap().parse().expect("Error parse value");
            if src.len() != 64 {
                println!("source address is invalid. Address can be only 32 byte hex string");
                return Err(());
            }
            if dst.len() != 64 {
                println!("destination address is invalid. Address can be only 32 byte hex string");
                return Err(());
            }
            let source_vec = hex::decode(src).expect("source addres is invalid hex string");
            let dest_vec = hex::decode(dst).expect("source addres is invalid hex string");

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

            println!("BoC succsessfully saved: {}", out);
        },
        "deploy" => {
            if !args.is_present("src") || !args.is_present("code") {
                println!("deploy smart-contract usage:");
                println!("--type deploy --src <src acc id> --code <code file name> --data <data file name> --out <output message>");
                return Err(());
            }
            let src: String = args.value_of("src").unwrap().parse().expect("Error parse source address");
            if src.len() != 64 {
                println!("source address is invalid. Address can be only 32 byte hex string");
                return Err(());
            }
            let source_vec = hex::decode(src).expect("source addres is invalid hex string");
            let code_file_name: String = args.value_of("code").unwrap().parse().expect("Error parse code file name");
            let code_str = std::fs::read_to_string(code_file_name).expect("Error read code file");
            
            let data_file_name: String = args.value_of("data").unwrap().parse().expect("Error parse data file name");
            let body_file_name: String = args.value_of("body").unwrap().parse().expect("Error parse body file name");

            let mut persistent_data = vec![];
            let mut file = File::open(data_file_name).expect("Error open data file");
            file.read_to_end(&mut persistent_data).expect("Error read data file");

            let mut body = vec![];
            let mut file = File::open(body_file_name).expect("Error open data file");
            file.read_to_end(&mut body).expect("Error read body file");
            let body = BuilderData::with_bitstring(body.clone()).unwrap().into();

            let message = MsgCreator::create_account_message(
                -1, 
                AccountId::from(source_vec),
                &code_str,
                BuilderData::with_bitstring(persistent_data).unwrap().into(),
                Some(body)
            );

            println!("Message = {:?}", message);
            
            let b = message.write_to_new_cell().expect("Error write message to tree of cells");
            let bag = BagOfCells::with_root(&b.into());

            let mut file = File::create(out.clone()).expect("Error create out file");
            bag.write_to(&mut file, false).expect("Error write message to file");
            file.flush().expect("Error flush out file");   

            println!("BoC succsessfully saved: {}", out);
        },
        _ => {
            println!("Invalid type option");
        }
    }

    Ok(())
}
