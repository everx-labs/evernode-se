# Giver v3

This directory contains Giver v3 (ABI v2.2) contract. This giver is recommended over Giver v1 or v2.

In Evernode SE this giver is predeployed (since version `TODO`) at `0:78fbd6980c10cf41401b32e9b51810415e7578b52403af80dae68ddf99714498` address 
and its initial balance is about 5 billion tokens. 

## Keys:

> ⚠ Using only local and dev environment

* Public: `2ada2e65ab8eeab09490e3521415f45b6e42df9c760a639bcf53957550b25a16`
* Secret: `172af540e43a524763dd53b26a066d472a97c4de37d5498170564510608250c3`

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
    --abi GiverV2.abi.json \
    --sign GiverV2.keys.json  
```

## Self deploy

### Compile
```shell
npx everdev sol set --compiler 0.59.4 --linker 0.15.24
npx everdev sol compile GiverV3.sol
```

> ℹ️ For compiler v0.59.4 and linker v0.15.2 code hash is `726aec999006a2e036af36c46024237acb946c13b4d4b3e1ad3b4ad486d564b1`

### Setup yore signer
```shell
npx everdev signer add <name_signer> <private_key>
npx everdev signer default <name_signer>
```

### Get `<giver_address>` and topup it
```shell
npx everdev contract info GiverV3
```

### Deploy
```shell
npx everdev contract deploy GiverV3
```

### Setup yore Giver
```shell
everdev network giver --signer <name_signer> --type GiverV3 <name_network> <giver_address>
```

## Files
* ABI: [GiverV3.abi.json](GiverV3.abi.json)
* Keypair: [GiverV3.keys.json](GiverV3.keys.json)
* Source code: [GiverV3.sol](GiverV3.sol)
* TVC file: [GiverV3.tvc](GiverV3.tvc)
