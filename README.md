# TON OS Startup Edition

Local blockchain for Free TON DApp development and testing.  

**Have a question? Get quick help in our channel:**

[![Channel on Telegram](https://img.shields.io/badge/chat-on%20telegram-9cf.svg)](https://t.me/ton_sdk) 

- [TON OS Startup Edition](#ton-os-startup-edition)
  - [What is TON OS Startup Edition?](#what-is-ton-os-startup-edition)
  - [Use-cases](#use-cases)
  - [How to install](#how-to-install)
    - [Pre-requisites](#pre-requisites)
    - [Instal via TONDEV Development Environment](#instal-via-tondev-development-environment)
    - [Install via docker command](#install-via-docker-command)
  - [How to change the blockchain configuration](#how-to-change-the-blockchain-configuration)
  - [How to work with logs](#how-to-work-with-logs)
  - [How to connect to TON OS SE Graphql API from SDK](#how-to-connect-to-ton-os-se-graphql-api-from-sdk)
  - [TON OS SE components](#ton-os-se-components)
  - [TON Live explorer](#ton-live-explorer)
  - [How to build docker image locally](#how-to-build-docker-image-locally)
    - [Linux/Mac:](#linuxmac)
    - [Windows:](#windows)


## What is TON OS Startup Edition?

TON OS Startup Edition (SE) is a local blockchain that developer can run on their machine in one click.   

At the moment we publish TON OS SE only as a [docker image](https://hub.docker.com/r/tonlabs/local-node). 
We plan to provide simple installers for MacOS, Win, Linux without docker by the end of Q1 2021.

See the [TON Labs TON OS SE documentation](https://docs.ton.dev/86757ecb2/p/19d886-ton-os-se) for detailed information.


## Use-cases

- Test your applications locally
- Test your contracts
- Run TON OS remotely on a server and test your application from different devices


## How to install

### Pre-requisites

- Latest [Docker](https://www.docker.com/get-started) installed

**Attention!** [Docker daemon](https://www.docker.com/get-started) must be running. 

### Instal via TONDEV Development Environment

If you have [TONDEV installed globally on your machine](https://github.com/tonlabs/tondev), run this command

```commandline
$ tondev se start
```
[Checkout other TON OS SE commands accessible from TONDEV](https://docs.ton.dev/86757ecb2/p/54722f-tonos-se). 
You can also access these commands from [TONDEV VS Code Extension](https://github.com/tonlabs/tondev-vscode).


### Install via docker command

Run this command 

```commandline
$ docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 tonlabs/local-node
```

To check that SE has been installed successfully check its local playground at http://0.0.0.0/graphql. 
For Windows, use http://127.0.0.1/graphql or http://localhost/graphql. 

If you specified another port then add it to the local url http://0.0.0.0:port/graphql

[Find out more about GraphQL API](https://docs.ton.dev/86757ecb2/p/793337-graphql-api). 

## How to change the blockchain configuration

TON OS SE loads the blockchain configuration (config params) during its start from the configuration file 
[blockchain.conf.json](docker/ton-node/blockchain.conf.json) instead of special smart contract, which stores 
various config params in the real networks.

In order to change some of these params, do the following:
1. Get [blockchain.conf.json](docker/ton-node/blockchain.conf.json) file and store it to the host's filesystem 
   accessible by docker. In our example we store it at `/home/user/blockchain.conf.json`.
2. Edit the downloaded file, changing parameters you need. If one of the parameters is omitted or renamed, 
   TON OS SE will not start.
3. Create a new docker container, overriding its configuration file 
   (its path in the image is `/ton-node/blockchain.conf.json`) with the file from the host's filesystem. 
   Change `/home/user/blockchain.conf.json` to correct path pointing to the edited blockchain configuration file:
```commandline
$ docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 \
     -v /home/user/blockchain.conf.json:/ton-node/blockchain.conf.json \
     tonlabs/local-node
```

## How to work with logs

By default, TON OS SE logs the most of the information to the console, which is accessible by the next command:
```commandline
$ docker logs local-node
```

More verbose logging is configured to `/ton-node/log/` directory inside the running docker container. 
By default, there are two files: `ton-node.log` for all logging and `tvm.log` for tracing of TVM execution: 
code, stack, control registers, gas, etc.

Logging configuration is stored in `/ton-node/log_cfg.yml` file. In order to change the default logging verbosity of 
other parameters, you can configure logging in several ways:
1. In the running container by changing `/ton-node/log_cfg.yml` file:

```commandline
$ docker exec -it local-node bash
bash-5.0# vi /ton-node/log_cfg.yml
```
(in order to exit from VI editor with saving changes press the `ESC` key, then type `:wq` and press the `ENTER` key)

Note: `log_cfg.yml` file is normally scanned for changes every 30 seconds, so all changes made to this file in running 
      container will be applied only after the scan.

Note: after recreation of the container, all changes made in its files will be lost, so use the second way, if you need 
      to keep them. 

2. Before starting of the container, download and edit a copy of [log_cfg.yml](./docker/ton-node/log_cfg.yml) file, then 
   mount this file to container's file system in `docker run` command:
```commandline
$ docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 \
     -v /home/user/log_cfg.yml:/ton-node/log_cfg.yml \
     tonlabs/local-node
```
After starting of TON OS SE, you can edit this file in your file system without restart.

More information about log4rs configuration [in the log4rs documentation](https://docs.rs/log4rs/1.0.0/log4rs/).


## How to connect to TON OS SE Graphql API from SDK

**Attention** at the moment there are a few [differences in SE behaviour comparing with a real TON blockchain](https://docs.ton.dev/86757ecb2/p/683279-difference-in-behaviour). Read about them before you start implemennting. Please note that we plan to upgrade the SE behaviour in the next releases so that it will work the same way as a real network.  

To connect to local blockchain from your application [specify localhost in SDK Client network config](https://docs.ton.dev/86757ecb2/p/5328db-tonclient).


## TON OS SE components

* [TON Labs implementation of TON VM written in Rust](https://github.com/tonlabs/ton-labs-vm)
* [ArangoDB database](https://www.arangodb.com/)
* [GraphQL endpoint with web playground](https://docs.ton.dev/86757ecb2/p/793337-graphql-api)
* [TON-live explorer](https://ton.live)
* [Pre-deployed high-performance Giver, ABI v2](contracts)


## TON Live explorer

TON Live explorer runs on the same IP and port as TON OS SE, just open http://ip_address:port (e.g. http://127.0.0.1)


## How to build docker image locally

In order to build and use TON OS Startup Edition you need Docker.
To build docker image, run from the repository root:


### Linux/Mac:

```commandline
./build.sh
```

### Windows:

```commandline
build.cmd
```
