extern crate tokio;
extern crate ton_node;

use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{ Arc };	
use tokio::prelude::Future;

use ton_node::adnl::{adnl_server::AdnlServer, AdnlOptions};
use ton_node::node_engine::ton_node_engine::TonNodeEngine;
use ton_node::node_engine::config::NodeConfig;

fn main() {
 
    println!("TON lite adnl server. v{}", env!("CARGO_PKG_VERSION"));
    
    env::set_current_dir(Path::new("../config")).unwrap();

    log4rs::init_file("./log_cfg.yml", Default::default())
        .expect("Error initialize logging configuration. config: log_cfg.yml");

    let config_file_name = "./cfg".to_string();
    let ton = TonNodeEngine::with_config(
        &config_file_name
    ).expect("Error create ton node engine");
    
    let ton = Arc::new(ton);

    // in normal situation, adnl server starting as a part of TonNodeEngine
    // but in this case we starts only adnl-server without devp2p
    let json = std::fs::read_to_string(Path::new(&config_file_name))
        .expect(&format!("Error reading config file {}", config_file_name));
    let adnl_port = NodeConfig::import_adnl_port(&json)
        .expect(&format!("Can`t get adnl port from config {}", config_file_name));
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), adnl_port);
    let config = Arc::new(NodeConfig::import_option(&json)
        .expect("Can`t get adnl option from config."));
                                    	
    AdnlServer::start_with_address(&addr, config, ton)
        .expect("Error start adnl server")
        .shutdown_on_idle()
        .wait()
        .unwrap();    

}
