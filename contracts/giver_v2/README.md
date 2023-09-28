# Giver v2

This directory contains Giver v2 (ABI v2) contract.

In Evernode SE this giver is predeployed at `0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415` address 
and its initial balance is about 5 billion tokens. 

It is not recommented to use in production or recompile Giver V2 because its works on old Solidity version.
If you want to make changes to the Giver contract or use it in production - use [Giver V3](../giver_v3) version which can be successfully compiled with the latest Solidity compiler. 

## Keys:
* Public: `2ada2e65ab8eeab09490e3521415f45b6e42df9c760a639bcf53957550b25a16`
* Secret: `172af540e43a524763dd53b26a066d472a97c4de37d5498170564510608250c3`

## Usage
Method: `sendTransaction`

parameters: 
* `dest`: `address` - destination address;
* `value`: `uint128` - amount to send, in nanotokens;
* `bounce`: `bool` - bounce flag of the message.

### Using everdev:
```commandline
everdev contract run GiverV2.abi.json sendTransaction --address 0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415 --signer seGiver --input dest:recipient_address,value:nanotokens,bounce:false,payload:""
```
For more information about `everdev` usage refer to 
[documentation](https://docs.everos.dev/everdev/command-line-interface/contract-management).



## Files
* ABI: [GiverV2.abi.json](GiverV2.abi.json)
* Keypair: [GiverV2.keys.json](GiverV2.keys.json)
* TVC file: [GiverV2.tvc](GiverV2.tvc)
* Source code: [GiverV2.sol](GiverV2.sol)
