# TON-Node
Telegram Open Network Blockchain Node implementation

## Prerequisites

Install Rust and building dependencies.

```commandline
sudo apt update && sudo apt install -y cmake pkg-config libssl-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh 
```

## To Build:

```commandline
cargo build --release
```

## To Test Node:

```
cargo test
```

## To Run:

### Startup Edition Node:
```
cargo run --release --bin ton_node_startup -- --workdir ./node0 --config ./config/cfg_startup
```
Don't forget, this kind of node needs properly configured Arango DB. Add db's address and collection's names to config file. 
