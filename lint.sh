#!/usr/bin/env bash

cargo +nightly fmt
cargo clippy --fix --allow-dirty
cargo fix --allow-dirty
