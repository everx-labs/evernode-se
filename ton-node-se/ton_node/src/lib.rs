#![recursion_limit = "128"]

// External 
extern crate bytes;
extern crate clap;
extern crate curve25519_dalek;
extern crate ed25519_dalek;
extern crate failure;
extern crate futures;
extern crate hex;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
extern crate log4rs;
extern crate num_traits;
extern crate parking_lot;
extern crate rand;
#[cfg(not(feature="local_node"))]
extern crate rdkafka;
extern crate serde;
extern crate serde_derive;
// #[macro_use]
extern crate serde_json;
extern crate stream_cancel;
extern crate tokio;
extern crate x25519_dalek;
extern crate tokio_io_timeout;

extern crate ton_vm as tvm;
extern crate ton_types;
extern crate ton_block;
extern crate ton_executor;
extern crate ton_labs_assembler;

// TBD
extern crate iron;
extern crate jsonrpc_http_server;
extern crate router;
extern crate threadpool;
extern crate num;

#[allow(deprecated)]
#[macro_use]
pub mod error;
pub mod node_engine;
