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

RUN TARGET=$( rustup target list --installed ) \
    ./build.sh

FROM gcr.io/distroless/cc-debian12 AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/rproxy ./

ENTRYPOINT [ "/app/rproxy" ]
