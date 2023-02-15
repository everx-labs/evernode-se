# Giver v3

This directory contains Giver v3 contract. This giver is recommended to use with solc version above 0.61.2 to deploy it on devnet or mainnet.

In Evernode SE this giver is predeployed at `0:ca600a7edcd15c13047b7b0bdf19c624f5c7fa3474d38afe323a373b0bdb30f5` address 
and its initial balance is about 5 billion tokens. 

## Keys:
* Public: `2ada2e65ab8eeab09490e3521415f45b6e42df9c760a639bcf53957550b25a16`
* Secret: `172af540e43a524763dd53b26a066d472a97c4de37d5498170564510608250c3`

## Usage
Method: `sendTransaction`

parameters: 
* `dest`: `address` - destination address;
* `value`: `uint128` - amount to send, in nanotokens;
* `bounce`: `bool` - bounce flag of the message.

### Using tonos-cli:
```commandline
tonos-cli call 0:ca600a7edcd15c13047b7b0bdf19c624f5c7fa3474d38afe323a373b0bdb30f5 \
    sendTransaction '{"dest":"<address>","value":<nanotokens>,"bounce":false}' \
    --abi GiverV3.abi.json \
    --sign seGiver.keys.json
```

## How to deploy GiverV3 on any network

### Setup your signer
You can skip this step if you already have one. 
```shell
npx everdev signer add devnet_giver_keys <private_key>
npx everdev signer default devnet_giver_keys
```

### [Optional] Verify Giver contract bytecode
This contract is compiled with `0.66.0 ` Solidity and `0.19.3`Linker version.

To check that the code hash of the compiled version from repository is equal to your freshly compiled version, run this command and check that the *Code Hash* is
`5534bff04d2d0a14bb2257ec23027947c722159486ceff9e408d6a4d796a0989`. 

```shell
npx everdev sol set --compiler 0.66.0 --linker 0.19.3
npx everdev sol compile GiverV3.sol
npx everdev contract info --signer devnet_giver_keys GiverV3

Configuration

  Network: se (http://localhost)
  Signer:  devnet_giver_keys (public 7fbbd813ac8358ed2d8598de156eb62bdddf5191d6ce4a0f307d4eac8d4c8e16)

Address:   0:dd39b607834a23f7091d4d6d8982c6269c1d71f1b512757cf4d298325a550b6a (calculated from TVC and signer public)
Code Hash: 5534bff04d2d0a14bb2257ec23027947c722159486ceff9e408d6a4d796a0989 (from TVC file)
```

### Get your Giver address and top it up
The address is calculated from the compiled contract codehash and your public key.
Run this command to see the *Address*:
```shell
npx everdev contract info --signer devnet_giver_keys GiverV3
```
Now, you need to top up your giver. Transfer tokens from Surf wallet or Everwallet.


### Deploy your Giver
After you topped up Giver address, you can deploy it.
Run this command:
```shell
npx everdev contract deploy GiverV3
```

### Setup your Giver
Run this command to set up the giver for your network. 

```shell
npx everdev network giver --signer <name_signer> --type GiverV3 <name_network> <giver_address>
```

### Using your Giver
This command under the hood will use predefined signer and configured giver on the default network.
```
npx everdev contract topup -a `<refill_address>` -v `<nano_tokens_value>`
```

## Files
* ABI: [GiverV3.abi.json](GiverV3.abi.json)
* Source code: [GiverV3.sol](GiverV3.sol)
* TVC file: [GiverV3.tvc](GiverV3.tvc)
