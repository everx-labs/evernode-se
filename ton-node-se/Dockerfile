# syntax=docker/dockerfile:1.0.0-experimental


###   STAGE build-kit
FROM alpine:latest as build-kit

RUN apk update; \
    apk add \
        bash bash-completion \
        cmake \
        clang clang-libs clang-dev \
        curl ca-certificates zlib-dev\
        gcc g++ \
        git \
        make \
        musl musl-utils musl-dev \
        openssh-client openssh-keygen openssh-keygen \
        openssl openssl-dev; \
        curl https://sh.rustup.rs -sSf | sh -s -- --default-toolchain stable -y; \
        source $HOME/.cargo/env; \
        rustup --version; \
        cargo --version; \
        rustc --version

RUN ssh-keygen -q -P "" -f ~/.ssh/id_rsa; \
    ssh-keyscan github.com >> ~/.ssh/known_hosts


###   STAGE cargo-init
FROM build-kit as cargo-init
ENV PKG_CONFIG_ALLOW_CROSS=1
ENV RUSTFLAGS="-C target-feature=-crt-static"
WORKDIR /ton-node
COPY . .


###   STAGE cargo-test
FROM cargo-init as cargo-test
ARG FEATURES="ci_run"

WORKDIR /ton-node/poa
RUN source $HOME/.cargo/env && cargo test

WORKDIR /ton-node/ton_node
RUN source $HOME/.cargo/env && cargo test --features ${FEATURES}


###   STAGE cargo-build
FROM cargo-init as cargo-build
RUN source $HOME/.cargo/env && cargo build --release


###   STAGE final-image
FROM alpine:3.10 as final-image

ARG BIN_TARGET

RUN apk update; \
    apk add libgcc libstdc++;

# ton-node
COPY --from=cargo-build \
    /ton-node/target/release/${BIN_TARGET} \
    /node/ton-node
COPY --from=cargo-build \
    /ton-node/config/log_cfg.yml \
    /ton-node/config/cfg_startup \
    /ton-node/config/key01 \
    /ton-node/config/pub01 \
    /ton-node/ton_node/entrypoint \
    /node/

# create-msg
COPY --from=cargo-build \
    /ton-node/target/release/create-msg \
    /node/create-msg/create-msg

WORKDIR /node

EXPOSE 3030
EXPOSE 30303

ENTRYPOINT ["./entrypoint"]
