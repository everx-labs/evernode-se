# syntax=docker/dockerfile:1.0.0-experimental


###   STAGE build-kit
FROM ubuntu:24.04 as build-kit

ENV CARGO_HOME=/rust
ENV RUST_HOME=/rust
ENV PATH="$RUST_HOME/bin:$PATH"

RUN apt update; \
    apt install -y \
        apt-utils \
        bash \
        bash-completion \
        build-essential \
        dos2unix \
        cmake \
        clang \
        dos2unix \
        libclang-dev \
        curl \
        ca-certificates \
        zlib1g-dev \
        gcc g++ \
        git \
        make \
        openssh-client \
        openssl \
        pkg-config \
        automake \
        sudo \
        libssl-dev; \
    groupmod -n jenkins ubuntu; \
    userdel -r ubuntu; \
    useradd -m -s /usr/bin/bash -u 1000 -g jenkins jenkins; \
    echo "jenkins  ALL=(ALL) NOPASSWD:ALL" >> /etc/sudoers; \
    mkdir $RUST_HOME; \
    curl https://sh.rustup.rs -sSf | sh -s -- --default-toolchain 1.80.1 -y; \
    . $RUST_HOME/env; \
    rustup --version; \
    cargo --version; \
    rustc --version

RUN ssh-keygen -q -P "" -f ~/.ssh/id_rsa; \
    ssh-keyscan github.com >> ~/.ssh/known_hosts

ENV PKG_CONFIG_ALLOW_CROSS=1
ENV RUSTFLAGS="-C target-feature=-crt-static"
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true

WORKDIR /ton-node
COPY . .

ARG FEATURES="disable-tests"

RUN dos2unix /ton-node/build.sh && \
    chmod +x /ton-node/build.sh && \
    /ton-node/build.sh $FEATURES

###   STAGE final-image
FROM ubuntu:24.04 as final-image

ARG BIN_TARGET

RUN apt update; \
    apt upgrade -y; \
    apt install -y libstdc++6 libgcc-14-dev;

# ton-node
COPY --from=build-kit \
    /ton-node/target/release/${BIN_TARGET} \
    /node/${BIN_TARGET}
COPY --from=build-kit \
    /ton-node/config/log_cfg.yml \
    /ton-node/config/cfg_startup \
    /ton-node/config/key01 \
    /ton-node/config/pub01 \
    /ton-node/entrypoint \
    /node/

WORKDIR /node

EXPOSE 3030
EXPOSE 30303

ENTRYPOINT ["./entrypoint"]
