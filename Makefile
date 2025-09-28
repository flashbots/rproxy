SHELL := /bin/bash

.PHONY: build
build:
	@cargo --verbose build

.PHONY: fmt
fmt:
	@rustfmt --config-path ./.rustfmt.toml --check ./crates/rproxy/**/*

.PHONY: help
help:
	@cargo run -- --help
