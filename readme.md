# TON OS Startup Edition

**Have a question? Get quick help in our channel:**

[![Channel on Telegram](https://img.shields.io/badge/chat-on%20telegram-9cf.svg)](https://t.me/ton_sdk) 

## What is TON OS Startup Edition?

TONOS Startup Edition (SE) is a local blockchain that developer can run on their machine in one click.  

At the moment we publish TONOS SE only as a [docker image](https://hub.docker.com/r/tonlabs/local-node). 
We plan to provide simple installers for MacOS, Win, Linux without docker by the end of Q1 2021.

See the [TON Labs TON OS SE documentation](https://docs.ton.dev/86757ecb2/p/19d886-ton-os-se) for detailed information.


## Use-cases
- Test your applications locally
- Test your contracts
- Run TONOS remotely on a server and test your application from different devices

## How to install
### Pre-requisites
- Latest [Docker](https://www.docker.com/get-started) installed

**Attention!** [Docker daemon](https://www.docker.com/get-started) must be running. 



Run this command 

```commandline
$ docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 tonlabs/local-node
```
To check that SE has been installed successfully check its local playground at http://0.0.0.0/graphql. 
For Windows, use http://127.0.0.1/ or http://localhost/graphql. 

[Find out more about GraphQL API](https://docs.ton.dev/86757ecb2/p/793337-graphql-api). 


## How to connect to TON OS SE Graphql API from SDK

[Specify localhost as a server in Client Libraries.](https://docs.ton.dev/86757ecb2/p/5328db-tonclient).


### TON OS SE components:

* [TON Labs implementation of TON VM written in Rust](https://github.com/tonlabs/ton-labs-vm)
* ArangoDB database,
* [GraphQL endpoint with web playground](https://docs.ton.dev/86757ecb2/p/793337-graphql-api). 
* [Pre-deployed Giver](https://docs.ton.dev/86757ecb2/p/00f9a3-ton-os-se-giver)


## How to build

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

## How to run

```commandline
docker run -d --name local-node -e USER_AGREEMENT=yes -p80:80 tonlabs/local-node
```

Container exposes the specified 80 port with nginx which proxies requests to /graphql to GraphQL API.

Check out GraphQL playground at http://localhost/graphql


## License

View [license](https://github.com/tonlabs/TON-SDK/blob/master/LICENSE) information for the software contained in this repository.
