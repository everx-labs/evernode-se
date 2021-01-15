#[macro_use]
extern crate lazy_static;
extern crate adnl;
extern crate clap;
extern crate hex;
extern crate log4rs;
extern crate tokio;
extern crate ton_api;
extern crate ton_block;
extern crate ton_vm as tvm;
extern crate ton_types;

use adnl::{config::AdnlClientConfig, client::AdnlClient};
use clap::{App, Arg};
use std::env;
use std::fs;
use std::io::Cursor;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process;
use std::sync::{Arc, Barrier, Mutex};
use std::{thread, time};
use tokio::prelude::Future;
use tokio::runtime::Runtime;
use ton_api::ton;
use ton_api::ton::TLObject;
use ton_api::ton::lite_server::{
    accountid::AccountId, AccountState, MasterchainInfo, SendMsgStatus,
};
use ton_api::ton::rpc::lite_server::{GetAccountState, GetMasterchainInfo, SendMessage};
use ton_block::Account;
use ton_block::{Deserializable};
use ton_types::cells_serialization::deserialize_cells_tree_ex;
use ton_types::SliceData;
use std::time::Duration;

lazy_static! {
    static ref LAST_BLOCK: Arc<Mutex<i32>> = Arc::new(Mutex::new(0));
}

fn tl_object_fail(object: TLObject, error_message: &'static str) {
    match object.downcast::<ton::lite_server::Error>() {
        Ok(error) => {
            println!("{}: {}", error_message, error.message());
        },
        Err(object) => {
            println!("{}. Unsupported answer from the node: {:?}", error_message, object);
        }
    }
}

fn do_get_account(adnl: Arc<Mutex<AdnlClient>>, param: &str) {
    if param.is_empty() {
        println!("Bad parameters");
        return;
    }
    if param.len() != 64 {
        println!("Bad address {}", param);
        return;
    }
    let buf = hex::decode(param).expect(&format!("Cannot decode address {}", param));
    let mut addr = [0u8; 32];
    addr.copy_from_slice(&buf[..]);
    let ping = Arc::new(Barrier::new(2));
    let pong = ping.clone();
    let data = GetAccountState {
        id: ton::ton_node::blockidext::BlockIdExt::default(),
        account: AccountId {
            workchain: 0,
            id: ton::int256(addr),
        },
    };
    adnl.lock()
        .unwrap()
        .query(&data, move |mut answer| {
            match answer.variant() {
                Ok(object) => {
                    match object.downcast::<AccountState>() {
                        Ok(account_state) => {
                            let (root_cells, _, _, _) =
                                deserialize_cells_tree_ex(&mut Cursor::new(&account_state.state().0)).expect("Error deserializing BOC");
                            let root_cells_vec: Vec<SliceData> =
                                root_cells.iter().map(|c| SliceData::from(c)).collect();
                            let root_cell = root_cells_vec[0].clone();
                            let mut account_decoded = Account::default();
                            let mut result = String::new();
                            account_decoded
                                .read_from(&mut SliceData::from(root_cell))
                                .expect("Cannot read from message slice");
                            let state = account_decoded.state().unwrap();
                            result += "Account status: ";
                            match state {
                                ton_block::AccountState::AccountActive(..) => {
                                    result += "AccountActive\n";
                                    result +=
                                        &format!("Code: {}\n", account_decoded.get_code().unwrap_or_default());
                                    result +=
                                        &format!("Data: {}\n", account_decoded.get_data().unwrap_or_default());
                                }
                                ton_block::AccountState::AccountFrozen(..) => {
                                    result += "AccountFrozen\n";
                                }
                                ton_block::AccountState::AccountUninit => {
                                    result += "AccountUninit\n";
                                }
                            }
                            result += &format!(
                                "Balance: {} nanograms\n",
                                account_decoded.get_balance().unwrap().grams.0
                            );
                            let stats = account_decoded.get_storage_stat().unwrap_or_default();
                            result += &format!(
                                "Storage used:\nCells: {}\nBits: {}\nPublic cells: {}\n",
                                stats.cells.0, stats.bits.0, stats.public_cells.0
                            );
                            println!("{}", result);
                        },
                        Err(object) => tl_object_fail(object, "Failed to get account information")
                    }
                },
                Err(_) => println!("Failed to get account information")
            };
            pong.wait();
        })
        .expect("Error sending GetAccountState query");
    ping.wait();
}

fn do_last(adnl: Arc<Mutex<AdnlClient>>, print_block : bool) {
    let ping = Arc::new(Barrier::new(2));
    let pong = ping.clone();
    let data = GetMasterchainInfo;
    adnl.lock()
        .unwrap()
        .query(&data, move |mut answer| {
            match answer.variant() {
                Ok(object) => {
                    match object.downcast::<MasterchainInfo>() {
                        Ok(masterchain_info) => {
                            let last_blk = masterchain_info.last();
                            if print_block {
                                println!("Last block:\n{}:{}:{}",
                                    last_blk.seqno,
                                    hex::encode(&last_blk.root_hash.to_vec()),
                                    hex::encode(&last_blk.file_hash.to_vec()));
                            }
                            *LAST_BLOCK.lock().unwrap() = masterchain_info.last().seqno;
                        },
                        Err(object) => tl_object_fail(object, "Failed to get last block")
                    }
                },
                Err(error) => println!("Failed to get last block: {}", error)
            }
            pong.wait();
        })
        .expect("Error sending GetMasterchainInfo query");
    ping.wait();
}

fn wait_for_block(adnl: Arc<Mutex<AdnlClient>>, timeout: &str) {
    let last = *LAST_BLOCK.lock().unwrap();
    let start = time::Instant::now();
    while last == *LAST_BLOCK.lock().unwrap() {
        let ping = Arc::new(Barrier::new(2));
        let pong = ping.clone();
        let data = GetMasterchainInfo;
        adnl.lock()
            .unwrap()
            .query(&data, move |mut answer| {
                match answer.variant() {
                    Ok(object) => {
                        match object.downcast::<MasterchainInfo>() {
                            Ok(masterchain_info) => *LAST_BLOCK.lock().unwrap() = masterchain_info.last().seqno,
                            Err(object) => tl_object_fail(object, "Failed to get masterchain info")
                        }
                    },
                    Err(error) => println!("Failed to get masterchain info: {}", error)
                }
                pong.wait();
            })
            .expect("Error sending GetMasterchainInfo query");
        ping.wait();
        println!("Waiting for a new block...");
        thread::sleep(time::Duration::from_millis(1000));
        if start.elapsed() > time::Duration::from_secs(timeout.parse::<u64>().unwrap_or(10)) {
            println!("Timeout expired");
            break;
        }
    }
}

fn do_send_file(adnl: Arc<Mutex<AdnlClient>>, param: &str) {
    if param.is_empty() {
        println!("Bad parameters");
        return;
    }
    let mut msg = Vec::new();
    fs::File::open(param)
        .expect(&format!("Cannot open file {}", param))
        .read_to_end(&mut msg)
        .expect(&format!("Cannot read file {}", param));
    
    let ping = Arc::new(Barrier::new(2));
    let pong = ping.clone();
    let data = SendMessage {
        body: ton::bytes(msg),
    };
    let file_name = Arc::new(param.to_owned());
    adnl.lock()
        .unwrap()
        .query(&data, move |mut answer| {
            match answer.variant() {
                Ok(object) => {
                    match object.downcast::<SendMsgStatus>() {
                        Ok(reply) => println!("Sendfile {} status: {}", file_name, reply.status()),
                        Err(object) => tl_object_fail(object, "Failed to get Sendfile status"),
                    }
                },
                Err(error) => println!("Failed to get Sendfile status: {}", error)
            };
            pong.wait();
        })
        .expect("Error sending SendMessage query");
    ping.wait();
}

const HELP: &str = "List of available commands:
    getaccount    <addr>      Loads the most recent state of specified account
    sendfile      <filename>  Loads a serialized message from <filename> and send it to server
    last                      Gets last block and state info from server
    wait                      Waits for a new block (applicable only for sendfile)
    quit or exit";

fn main() {
    println!("ADNL client {}\nCOMMIT_ID: {}\nBUILD_DATE: {}\nCOMMIT_DATE: {}\nGIT_BRANCH: {}",
            env!("CARGO_PKG_VERSION"),
            env!("BUILD_GIT_COMMIT"),
            env!("BUILD_TIME") ,
            env!("BUILD_GIT_DATE"),
            env!("BUILD_GIT_BRANCH"));
    let mut command_queue: Vec<(&str, &str)> = Vec::new();

    let args = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .arg(
            Arg::with_name("workdir")
                .help("Path to working directory")
                .long("workdir")
                .short("W")
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("config")
                .help("Configuration file name")
                .long("config")
                .short("c")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("wait")
                .help("Wait until a new block is generated, applicable only for sendfile")
                .long("wait")
                .short("w")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("timeout-wait")
                .help("Timeout for wait option")
                .long("wait-timeout")
                .short("t")
                .required(false)
                .takes_value(false),
        )
        .arg(
            Arg::with_name("sendfile")
                .help("Deploy BOC")
                .long("sendfile")
                .short("s")
                .required(false)
                .takes_value(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("getaccount")
                .help("Getaccount information")
                .long("getaccount")
                .short("g")
                .required(false)
                .takes_value(true)
                .multiple(true),
        )
        .arg(
            Arg::with_name("last")
                .help("Get last block and state info from server")
                .long("last")
                .short("l")
                .required(false)
                .takes_value(false),
        )
        .get_matches();

    let wait_enabled = args.is_present("wait");
    if args.is_present("sendfile") {
        let values = args.values_of("sendfile").unwrap().collect::<Vec<_>>();
        for i in values {
            command_queue.push(("sendfile", i));
            if wait_enabled {
                command_queue.push(("wait", ""));
            }
        }
    }

    if args.is_present("getaccount") {
        let values = args.values_of("getaccount").unwrap().collect::<Vec<_>>();
        for i in values {
            command_queue.push(("getaccount", i));
        }
    }

    let timeout = args.value_of("wait-timeout").unwrap_or("10");

    if args.is_present("workdir") {
        if let Some(workdir) = args.value_of("workdir") {
            env::set_current_dir(Path::new(workdir))
                .expect(&format!("Cannot set working directory to {}", workdir))
        }
    }

    let logcfg = "./log_cfg.yml";
    log4rs::init_file(logcfg, Default::default()).expect(&format!(
        "Cannot read logging configuration from {}",
        logcfg
    ));

    let appcfg = args.value_of("config").unwrap();
    let config = AdnlClientConfig::from_json(
        &fs::read_to_string(Path::new(appcfg))
            .expect(&format!("Cannot read ADNL confiuration from {}", appcfg)),
    );

    let adnl = Arc::new(Mutex::new(AdnlClient::new()));
    let network_timeout = Some(Duration::from_secs(20));
    let future = adnl
        .lock()
        .unwrap()
        .connect(&config, network_timeout, network_timeout)
        .map_err(|error| {
            println!("Error while establishing connection: {:?}", error);
            process::exit(-1);
        });
    let mut rt = Runtime::new().unwrap();
    rt.spawn(future);

    if args.is_present("last") {
        command_queue.push(("last", ""));
    }
    else {
        do_last(adnl.clone(), false);
    }

    let match_commands = |commands: &Vec<(&str, &str)>| {
        for cmd in commands {
            match cmd.0 {
                "help" => println!("{}", HELP),
                "wait" => wait_for_block(adnl.clone(), timeout),
                "getaccount" => do_get_account(adnl.clone(), cmd.1),
                "last" => do_last(adnl.clone(), true),
                "sendfile" => do_send_file(adnl.clone(), cmd.1),
                "quit" | "exit" => {
                    process::exit(0);
                }
                _ => println!("Bad command"),
            }
        }
    };

    if command_queue.len() > 0 {
        command_queue.push(("exit", ""));
        match_commands(&command_queue);
    } else {
        println!("{}", HELP);
        loop {
            print!("Enter command > ");
            io::stdout().flush().expect("I/O error");
            let mut input = String::new();
            io::stdin()
                .read_line(&mut input)
                .expect("Unable to read user input");
            let mut cmds = vec![];
            let mut s = input.split_whitespace();
            while let Some(cmd) = s.next() {
                match s.next() {
                    Some(param) => cmds.push((cmd, param)),
                    None => cmds.push((cmd.trim_end(), "")),
                }
            }
            match_commands(&cmds);
        }
    }
    rt.shutdown_on_idle().wait().unwrap();
}
