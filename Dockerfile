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

RUN CARGO_INCREMENTAL=0 \
    CFLAGS="-D__TIME__=\"\" -D__DATE__=\"\"" \
    CXXFLAGS="-D__TIME__=\"\" -D__DATE__=\"\"" \
    LC_ALL=C \
    RUSTFLAGS="--cfg tracing_unstable -C metadata=host" \
    SOURCE_DATE_EPOCH=$(git log -1 --pretty=%ct) \
    TZ=UTC \
    cargo build \
        --release \
        --locked \
        --package rproxy

FROM gcr.io/distroless/cc-debian12 AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/rproxy ./

ENTRYPOINT [ "/app/rproxy" ]
