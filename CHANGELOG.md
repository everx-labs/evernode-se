# Release Notes
All notable changes to this project will be documented in this file.

## 0.26.1 Apr 09, 2021
### Fixed
- Transaction can be lost if it is created near the end of block producing interval.

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

