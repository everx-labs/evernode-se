#!/bin/bash -xe

FEATURES=$1

source "$HOME/.cargo/env"

if [ "$FEATURES" != "disable-tests" ]; then
  cargo test --release
fi

cargo build --release
