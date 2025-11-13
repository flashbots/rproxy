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

RUN ./build.sh
RUN cp target/$( rustc --print host-tuple )/release/rproxy target/rproxy

FROM gcr.io/distroless/cc-debian12 AS runtime

WORKDIR /app

COPY --from=builder /app/target/rproxy ./

ENTRYPOINT [ "/app/rproxy" ]
