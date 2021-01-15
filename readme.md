# TON OS Startup Edition

## What is TON OS Startup Edition?

TONOS Startup Edition (SE) or Local Node is a pre-configured Docker image with a local blockchain that provides the same GraphQL API as a Dapp Server.

Designed for Dapp debugging and testing.

### TON OS SE consists of:

* TON Labs implementation of TON VM written in Rust
* ArangoDB database,
* GraphQL endpoint with web playground. Learn more here.
* Pre-deployed Giver

See the TON Labs TON OS SE documentation for detailed information.

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

## Connect to TON OS SE Graphql API from SDK

Specify localhost as a server in Client Libraries. See the [documentation](https://docs.ton.dev/86757ecb2/p/5328db-tonclient).

## License

View [license](https://github.com/tonlabs/TON-SDK/blob/master/LICENSE) information for the software contained in this repository.