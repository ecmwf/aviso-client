# SPDX-FileCopyrightText: 2026 European Centre for Medium-Range Weather Forecasts (ECMWF)
# SPDX-License-Identifier: Apache-2.0

# ────────────────────────────────────────────────────────────────
# aviso-client CI image  ->  eccr.ecmwf.int/aviso/cli-ci
#
# Pre-bakes the Rust toolchain, sccache, uv, and the doc/lint/packaging
# tools (mdbook, mdbook-mermaid, cargo-deny, cargo-cinstall) as
# prebuilt static binaries.
# A system python3 is baked so workspace cargo commands can build the
# aviso-py PyO3 crate (pyo3 abi3 + auto-initialize need an interpreter
# and libpython). Version-specific interpreters for the binding test
# matrix are still installed at CI runtime by uv.
#
#   docker build -t aviso-cli-ci:test .github/ci/
# ────────────────────────────────────────────────────────────────

# Bump these to refresh the baked tools, then bump .github/ci/VERSION.
ARG UV_VERSION=0.11.17
ARG RUST_VERSION=1.95.0
ARG SCCACHE_VERSION=0.15.0
ARG MDBOOK_VERSION=0.5.3
ARG MDBOOK_MERMAID_VERSION=0.17.0
ARG CARGO_DENY_VERSION=0.19.8
ARG CARGO_C_VERSION=0.10.23

FROM ghcr.io/astral-sh/uv:${UV_VERSION} AS uv

FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive \
    LANG=C.UTF-8 \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# gcc + libc6-dev: link Rust binaries and build ring/cc (the only
# native deps in Cargo.lock). git: checkout + the Cargo.lock-diff gate.
# ca-certificates: TLS to crates.io and the sccache S3 bucket.
# python3-dev: the interpreter + libpython that pyo3 (abi3 +
# auto-initialize, extension-module off) needs to build and run
# aviso-py under workspace cargo commands.
# g++ + make + cmake: build the aviso-ffi C++ example and header-only
# facade against the prebuilt C ABI (the C++ binding CI job).
# pkg-config: lets a consumer locate the aviso_ffi.pc that
# `cargo cinstall` lays down (the FFI install-contract CI job).
# No OpenSSL, node or wasm: nothing here needs them (reqwest is
# rustls, ring drives gcc directly).
RUN set -eux \
 && apt-get update \
 && apt-get install -y --no-install-recommends \
      gcc g++ make cmake libc6-dev git curl ca-certificates pkg-config \
      python3-dev \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* \
           /usr/share/doc /usr/share/man /usr/share/info

# Rust toolchain pinned to rust-toolchain.toml. rustup default avoids a
# re-resolve when cargo reads rust-toolchain.toml from a checkout.
ARG RUST_VERSION
RUN set -eux \
 && curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --default-toolchain "${RUST_VERSION}" \
                 --profile minimal -c clippy -c rustfmt \
 && rustup default "${RUST_VERSION}" \
 && rm -rf "${RUSTUP_HOME}"/toolchains/*/share/doc \
           "${RUSTUP_HOME}"/toolchains/*/share/man /tmp/*

# Prebuilt static (musl) binaries: smaller than compiling from source
# and the image build stays in seconds, not minutes.
ARG SCCACHE_VERSION
ARG MDBOOK_VERSION
ARG MDBOOK_MERMAID_VERSION
ARG CARGO_DENY_VERSION
ARG CARGO_C_VERSION
RUN set -eux \
 && cd /usr/local/bin \
 && curl -fsSL "https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz --strip-components=1 "sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" \
 && curl -fsSL "https://github.com/EmbarkStudios/cargo-deny/releases/download/${CARGO_DENY_VERSION}/cargo-deny-${CARGO_DENY_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz --strip-components=1 "cargo-deny-${CARGO_DENY_VERSION}-x86_64-unknown-linux-musl/cargo-deny" \
 && curl -fsSL "https://github.com/rust-lang/mdBook/releases/download/v${MDBOOK_VERSION}/mdbook-v${MDBOOK_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz mdbook \
 && curl -fsSL "https://github.com/badboy/mdbook-mermaid/releases/download/v${MDBOOK_MERMAID_VERSION}/mdbook-mermaid-v${MDBOOK_MERMAID_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz mdbook-mermaid \
 && curl -fsSL "https://github.com/lu-zero/cargo-c/releases/download/v${CARGO_C_VERSION}/cargo-c-x86_64-unknown-linux-musl.tar.gz" \
    | tar xz cargo-cinstall

COPY --from=uv /uv /uvx /usr/local/bin/

RUN set -eux \
 && rustc --version && cargo --version \
 && cargo clippy --version && rustfmt --version \
 && mdbook --version && mdbook-mermaid --version && cargo-deny --version \
 && cargo cinstall --version \
 && pkg-config --version \
 && uv --version \
 && sccache --version \
 && python3 --version

WORKDIR /workspace
