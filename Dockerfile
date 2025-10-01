FROM rust:1.90-slim-bookworm AS base

RUN apt-get update && \
    apt-get install --yes \
        clang \
        libclang-dev

RUN rustup component add \
    clippy \
    rustfmt

ENV CARGO_HOME=/usr/local/cargo

FROM base AS builder

WORKDIR /app

COPY . .
COPY ./.cargo ./

ENV CARGO_INCREMENTAL="0"
ENV CFLAGS="-D__TIME__=\"\" -D__DATE__=\"\""
ENV CXXFLAGS="-D__TIME__=\"\" -D__DATE__=\"\""
ENV LC_ALL="C"
ENV RUSTFLAGS="--cfg tracing_unstable -C metadata=host"
ENV TZ="UTC"

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="\
        --cfg tracing_unstable \
        --remap-path-prefix /app=. \
        -C codegen-units=1 \
        -C embed-bitcode=no \
        -C link-arg=-static-libgcc \
        -C link-arg=-Wl,--build-id=none \
        -C metadata=target \
        -C target-feature=+crt-static \
    "

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="\
        --cfg tracing_unstable \
        --remap-path-prefix /app=. \
        -C codegen-units=1 \
        -C embed-bitcode=no \
        -C link-arg=-static-libgcc \
        -C link-arg=-Wl,--build-id=none \
        -C metadata=target \
        -C target-feature=+crt-static \
    "

RUN SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) \
    cargo build --package rproxy \
        --release \
        --locked

FROM gcr.io/distroless/cc-debian12 AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/rproxy ./

ENTRYPOINT [ "/app/rproxy" ]
