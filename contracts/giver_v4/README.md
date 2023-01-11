# Giver v4

This directory contains Giver v4 (ABI v2.2) contract. This giver is recommended to use with solc version above 0.61.2 to deploy it on devnet or mainnet.

## Usage
Method: `sendTransaction`

parameters: 
* `dest`: `address` - destination address;
* `value`: `uint128` - amount to send, in nanotokens;
* `bounce`: `bool` - bounce flag of the message.

### Using tonos-cli:
```shell
npx tonos-cli call 0:78fbd6980c10cf41401b32e9b51810415e7578b52403af80dae68ddf99714498 \
    sendTransaction '{"dest":"<address>","value":<nanotokens>,"bounce":false}' \
    --abi GiverV4.abi.json \
    --sign <private_key>  
```

## Self deploy

### Compile
```shell
npx everdev sol set --compiler 0.61.2 --linker 0.17.3
npx everdev sol compile GiverV4.sol
```

### Setup your signer
```shell
npx everdev signer add <name_signer> <private_key>
npx everdev signer default <name_signer>
```

### Get `<giver_address>` and topup it
```shell
npx everdev contract info GiverV4
```

### Deploy
Before deploy you need to transfer some tokens on address (calculated from TVC and signer public)
```shell
npx everdev contract deploy GiverV4
```

### Setup your Giver
Type of this giver same as GiverV3
```shell
everdev network giver --signer <name_signer> --type GiverV3 <name_network> <giver_address>
```

## Files
* ABI: [GiverV4.abi.json](GiverV4.abi.json)
* Source code: [GiverV4.sol](GiverV4.sol)
