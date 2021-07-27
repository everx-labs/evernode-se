# Release Notes
All notable changes to this project will be documented in this file.

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

