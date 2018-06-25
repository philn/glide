#!/bin/sh

export CARGO_HOME=$1/target/cargo-home

if [[ $DEBUG = true ]]
then
    echo "DEBUG MODE"
    cargo build --manifest-path $1/Cargo.toml -p glide && cp $1/target/debug/glide $2
else
    echo "RELEASE MODE"
    cargo build --manifest-path $1/Cargo.toml --release -p glide && cp $1/target/release/glide $2
fi
