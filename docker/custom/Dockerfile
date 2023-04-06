###   STAGE build-kit
FROM alpine:3.14 as build-kit
ENV RUST_VERSION=1.65.0
ENV CARGO_HOME=/rust

RUN apk update && \
    mkdir $CARGO_HOME && \
    apk add \
        bash \
        bash-completion \
        ca-certificates \
        clang \
        clang-dev \
        clang-libs \
        cmake \
        curl \
        g++ \
        gcc \
        git \
        make \
        musl \
        musl-dev \
        musl-utils \
        openssh-client \
        openssh-keygen \
        openssl \
        openssl-dev \
        zlib-dev && \
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

RUN cargo build --release

###   STAGE final-image
FROM alpine:3.10 as final-image
WORKDIR /ton-node

RUN apk update; \
    apk add libgcc libstdc++;

# ton-node
COPY --from=build-kit /ton-node/target/release/ton_node_startup /
COPY ./docker/custom/entrypoint /
COPY ./docker/ton-node .

EXPOSE 3000

ENTRYPOINT ["/entrypoint"]
