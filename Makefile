SHELL := /bin/bash

.PHONY: build
build:
	@cargo --verbose build

.PHONY: fmt
fmt:
	@cargo +nightly fmt --check

.PHONY: lint
lint:
	@cargo +nightly clippy --all-features -- -D warnings

.PHONY: help
help:
	@cargo run -- --help

.PHONY: docker
docker:
	@docker build -t rproxy --progress plain .
