# Release Notes
All notable changes to this project will be documented in this file.

## 0.36.0 Dec 7, 2022

### New

- `CapFullBodyInBounced` is enabled to put full body in bounced message
- `CapCopyleft` is enabled to use contracts with copyleft instructions (but does not work fully yes, masterchain is not yet supported in SE)

### Fixed
- Account balance was not updated after account destruction

## 0.35.1 Nov 14, 2022

### New
- Max ext_in_msg size is 64kb

## 0.35.0 Oct 13, 2022

### New

- Blockchain API is supported (except `blockchain{ key_blocks }`)

### Fixed

- Incorrect inner message order
- Account with `nonExist` acc_type was created upon a failed deploy

## 0.34.0 Sep 13, 2022

### New
- Gosh VM instructions are supported:
  - execute_diff
  - execute_diff_patch_not_quiet
  - execute_zip
  - execute_unzip
  - execute_diff_zip
  - execute_diff_patch_zip_not_quiet
  - execute_diff_patch_quiet
  - execute_diff_patch_zip_quiet
  - execute_diff_patch_binary_not_quiet
  - execute_diff_patch_binary_zip_not_quiet
  - execute_diff_patch_binary_quiet
  - execute_diff_patch_binary_zip_quiet

## 0.33.1 Aug 24, 2022

### Improved
- Inner refactoring that increases message processing speed by 70-400% depending on the test logic.

## 0.33.0 Aug 04, 2022

### New
- `/se/time-delta` returns current `gen_time_delta` property value.

## 0.32.0 Jul 06, 2022

### New
- Block producing stopped if million of gas is consumed
- Waiting of new external messages added
- Block builder processes internal messages in the same block

## 0.31.0 Jun 20, 2022

### New

- `log_path` config field for configuring node log file location. 
- `/se` REST endpoint for SE realtime control. See [README.md](README.md#se-live-control-rest-api).
- `/se/increase-time?delta=<seconds>` feature to move time forward. See [README.md](README.md#se-live-control-rest-api)
- PoA consensus was removed from source code.
- Source code drastically simplified and reorganised.
- Randomization added for block generation
- Extra thread creation was removed
- Tokio crate dependencies were removed
- Extra crate dependencies were removed

### Fixed

- tvm.random() now generates random values 

## 0.30.2 May 3, 2022
### New
- Build with new version q-server 0.51.0

## 0.30.1 Feb 30, 2022
### New
- Build with new version q-server 0.49.0

## 0.30.0 Feb 17, 2022
### New
- Account has new field `init_code_hash`
- `q-server` 0.47.0 with `X-Evernode-Expected-Account-Boc-Version` header support

## 0.29.1 Feb 17, 2022
### Fixed
- Build with new version q-server 0.46.0

## 0.29.0 Feb 09, 2022
### New
- `MYCODE` VM instruction supported

## 0.28.12 Jan 26, 2022
### Fixed
- Support breaking changes in `ton-labs-block-json` v0.7.1

## 0.28.11 Nov 01, 2021
### Fixed
- Fixed honcho version in order to fix container start (ignoring incompatible release).

## 0.28.10 Oct 13, 2021
### Fixed
- Internal fixes in order to fix building.

## 0.28.9 Sep 24, 2021
### Fixed
- Internal fixes in order to fix building.

## 0.28.8 Sep 22, 2021
### Fixed
- Build with new version q-server 0.43.0

## 0.28.7 Sep 5, 2021
### Fixed
- Predeployed fixed version of Giver V2 (with the new address). The old version of giver left at the same old address 
  for backward compatibility.

## 0.28.6 Jul 27, 2021
### Fixed
- Removed useless adnl to fix the build process.

## 0.28.5 Jul 19, 2021
### Fixed
- Build with new version q-server 0.41.0

## 0.28.4 Jul 07, 2021
### Fixed
- [tvm.rawReserve](https://github.com/tonlabs/TON-Solidity-Compiler/blob/master/API.md#tvmrawreserve) function now correctly reacts on all flags.

## 0.28.3 May 20, 2021
### Fixed
- Made code ready for planned updates in dependant repositories

## 0.28.2 May 18, 2021
### Fixed
- Fixed previous bugfix. If you still have an error running TON Live explorer, clear your browser cache. 

## 0.28.1 May 18, 2021
### Fixed
- reloading any page (except `/` and `/landing` page) failed with 404 error

## 0.28.0 May 15, 2021
### New
- Predeployed [SafeMultisigWallet](contracts/safe_multisig) contract with 1 million tokens.
- [Improved logging. Added TVM log (tvm.log file)](README.md#how-to-work-with-logs).
### Fixed
- Contract freezing on receiving bounceable message when balance is zero.
- Crashes of logging in docker.

## 0.27.2 Apr 28, 2021
### Fixed
-  Transaction could be lost if it was created near the end of block producing interval. Again

## 0.27.1 Apr 28, 2021
### Fixed
- Subscriptions for blocks, transactions and messages do not trigger multiple times any more.

## 0.27.0 Apr 20, 2021
### New
- Support of blockchain config parameters.
- Ability to change the default blockchain config parameters.
- [TON live explorer](https://ton.live) running on the same IP and port as TON OS SE, just open http://ip_address:port (e.g. http://127.0.0.1).   
  You can explore blocks, transactions, accounts and messages of your TON OS SE 

## 0.26.1 Apr 14, 2021
### Fixed
- Transaction could be lost if it was created near the end of block producing interval.

## 0.26.0 Apr 08, 2021
### Fixed
- External inbound messages now have `created_at` field filled.

## 0.25.0 Mar 05, 2021
### New
- [New high-load Giver with ABI Version 2](contracts) is supported. Now you can run your tests in parallel, giver supports up to 100 parallel requests at a time (previous giver had timestamp-based replay protection which didn't allow often access)

## 0.24.13 Feb 26, 2021
### Fixed
- Documentation update

## 0.24.12 Feb 18, 2021
### Fixed
- Internal messages are sent in correct order regarding LT

## 0.24.11 Jan 29, 2021
### Fixed
- Internal Server Error 500 on some queries. Hangs during deployment of some contracts.

## 0.24.10 Jan 22, 2021
### Fixed
- Bounced message is now sent properly
