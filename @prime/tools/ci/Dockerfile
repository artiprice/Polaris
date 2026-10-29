# Polaris CI image for /ci-run (P8): the pinned Rust toolchain with the components the default
# Rust pipeline needs (cargo fmt, clippy). Build once: docker build -t polaris-ci-rust:1.99.0 @prime/tools/ci
FROM rust:1.99.0-bookworm
RUN rustup component add rustfmt clippy
