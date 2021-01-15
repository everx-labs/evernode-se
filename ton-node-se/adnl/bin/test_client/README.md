#### List of available commands:
```
    getaccount    <addr>      Loads the most recent state of specified account
    sendfile      <filename>  Loads a serialized message from <filename> and send it to server
    last                      Gets last block and state info from server
    wait                      Waits for a new block (applicable only for sendfile)
    quit or exit";
```

#### Short commands:
```
    getaccount = -g
    sendfile   = -s
    last       = -l
    wait       = -w
```

### Examples of usage
1) Combine different commands
```
cargo run --example adnl_client_test -- \
  --workdir ./adnl/examples \
  --config ./config \
  -s sendmoney.boc \
  -w -s msg-init.boc \
  -g 7f2b8e164b00af8e1ed9455cbe354f4b208bd895690472262a61e056908f32ed
```

2) Interactive mode with help message
```
cargo run --example adnl_client_test -- --workdir ./adnl/examples --config ./config
```