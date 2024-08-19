#!/bin/bash -xe

FEATURES=$1

if [ "$FEATURES" != "disable-tests" ]; then
  cargo test --release
fi

cargo build --release
