# Giver v4

This directory contains Giver v4 (ABI v2.2) contract. This giver is recommended to use with solc version above 0.61.2 to deploy it on devnet or mainnet.

## Self deploy contract on any network

### Compile Giver contract
This contract is prepared for compilation by `solc_0_61_2` and `tvm_linker_0_17_3` or above.
```shell
npx everdev sol set --compiler 0.61.2 --linker 0.17.3
npx everdev sol compile GiverV4.sol
```

### Setup your signer
You can skip this step if you already have one. But make sure the right signer linked to `<name_network>`.
```shell
npx everdev signer add <name_signer> <private_key>
npx everdev signer default <name_signer>
```

### Get your Giver `<giver_address>`
The address was calculated using the compiled contract codehash and your public key.
```shell
npx everdev contract info GiverV4
```

### Deploy your Giver
Before deploy you need to transfer tokens to `<giver_address>`.
```shell
npx everdev contract deploy GiverV4
```

### Setup your Giver
Before this step, make sure that you have configured the network with the correct endpoint, which contains your evercloud projectId.
```shell
npx everdev network giver --signer <name_signer> --type GiverV4 <name_network> <giver_address>
```

### Using your Giver
This command under the hood will use predefined signer and configured giver on the default network.
```
npx everdev contract topup -a `<refill_address>` -v `<nano_tokens_value>`
```

## Files
* ABI: [GiverV4.abi.json](GiverV4.abi.json)
* Source code: [GiverV4.sol](GiverV4.sol)
* TVC file: [GiverV4.tvc](GiverV4.tvc)
