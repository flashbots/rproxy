#!/bin/bash

export CARGO_INCREMENTAL=0
export CFLAGS="-D__TIME__=\"\" -D__DATE__=\"\""
export CXXFLAGS="-D__TIME__=\"\" -D__DATE__=\"\""
export LC_ALL=C
export SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct)
export TZ=UTC

TARGET=${TARGET:-x86_64-unknown-linux-gnu}

export RUSTFLAGS="\
  --cfg tracing_unstable \
  -C metadata=host \
"

export CARGO_TARGET_$( echo "${TARGET}" | tr '[:lower:]' '[:upper:]' | tr '-' '_' )_RUSTFLAGS="\
  --cfg tracing_unstable \
  --remap-path-prefix /app=. \
  -C codegen-units=1 \
  -C embed-bitcode=no \
  -C link-arg=-static-libgcc \
  -C link-arg=-Wl,--build-id=none \
  -C metadata=target \
  -C target-feature=+crt-static \
"

cargo build --package rproxy \
  --locked \
  --release \
  --target ${TARGET}
