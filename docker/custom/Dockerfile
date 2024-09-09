###   STAGE build-kit
FROM ubuntu:24.04 AS build-kit
ENV RUST_VERSION=1.80.1
ENV CARGO_HOME=/rust

RUN apt update && \
    mkdir $CARGO_HOME && \
    apt install -y \
        bash \
        bash-completion \
        ca-certificates \
        clang \
        libclang-dev \
        cmake \
        curl \
        g++ \
        gcc \
        git \
        make \
        pkg-config \
        openssh-client \
        openssl \
        libssl-dev \
        zlib1g-dev && \
    curl https://sh.rustup.rs -sSf | sh -s -- --default-toolchain $RUST_VERSION -y && \
    ssh-keygen -q -P "" -f ~/.ssh/id_rsa && \
    ssh-keyscan github.com >> ~/.ssh/known_hosts;

ENV PKG_CONFIG_ALLOW_CROSS=1
ENV RUSTFLAGS="-C target-feature=-crt-static"
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV PATH="$CARGO_HOME/bin:${PATH}"

WORKDIR /ton-node
COPY Cargo.toml .
COPY Cargo.lock .
COPY ./node ./node
COPY ./integration-tests ./integration-tests

RUN cargo build --release

###   STAGE final-image
# FROM alpine:3.20.2 AS final-image
FROM ubuntu:24.04 AS final-image
WORKDIR /ton-node

RUN apt update; \
    apt install -y libgcc-14-dev libstdc++-14-dev;

# ton-node
COPY --from=build-kit /ton-node/target/release/evernode_se /
COPY ./docker/custom/entrypoint /
COPY ./docker/ton-node .

EXPOSE 3000

ENTRYPOINT ["/entrypoint"]
