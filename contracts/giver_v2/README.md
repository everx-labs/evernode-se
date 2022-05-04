# Giver v2

This directory contains Giver v2 (ABI v2) contract. This giver is recommended over Giver v1 because ABI v1 is deprecated.

In Evernode SE this giver is predeployed at `0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415` address 
and its initial balance is about 5 billion tokens. 

If you want to compile and deploy your own v2 giver you can find its fresh version in `giver_for_Sol_0.59.0_abi.2.2` folder that is compatible with the latest compiler version. We do not deploy this giver to SE because it has the same ABI, but will cause a breaking change. 

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
tonos-cli call 0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415 \
    sendTransaction '{"dest":"<address>","value":<nanotokens>,"bounce":false}' \
    --abi GiverV2.abi.json \
    --sign GiverV2.keys.json  
```


## Files
* ABI: [GiverV2.abi.json](GiverV2.abi.json)
* Keypair: [GiverV2.keys.json](GiverV2.keys.json)
* TVC file: [GiverV2.tvc](GiverV2.tvc)
* Source code: [GiverV2.sol](GiverV2.sol)
