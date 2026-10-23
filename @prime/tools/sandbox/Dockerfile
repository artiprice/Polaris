ARG BASE_IMAGE=nvcr.io/nvidia/base/ubuntu:24.04
FROM rust:1.99.0-bookworm AS polaris-guest-build
WORKDIR /workspace
# build-wsl.sh passes only the allowlisted context produced by build-context.py.
COPY . /workspace
RUN cargo build --locked --release -p polaris-guest --bin polaris-guest

ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates coreutils build-essential pkg-config git curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 sandbox \
    && install -d -o sandbox -g sandbox /sandbox
COPY --from=polaris-guest-build /usr/local/cargo/bin /usr/local/cargo/bin
COPY --from=polaris-guest-build /usr/local/cargo/registry /sandbox/cargo/registry
COPY --from=polaris-guest-build /usr/local/cargo/git /sandbox/cargo/git
COPY --from=polaris-guest-build /usr/local/rustup /usr/local/rustup
ENV PATH="/usr/local/cargo/bin:${PATH}"
ENV CARGO_HOME=/sandbox/cargo
ENV CARGO_TARGET_DIR=/sandbox/build
ENV CARGO_NET_OFFLINE=true
ENV RUSTUP_HOME=/usr/local/rustup
ENV RUSTUP_TOOLCHAIN=1.99.0
ENV RUSTUP_AUTO_INSTALL=0
COPY --from=polaris-guest-build /workspace/target/release/polaris-guest /usr/bin/polaris-guest
RUN chmod 0755 /usr/bin/polaris-guest \
    && install -d -o sandbox -g sandbox /sandbox/cargo \
    && chown -R sandbox:sandbox /sandbox/cargo \
    && test -x /usr/local/rustup/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin/rustc \
    && test -x /usr/local/rustup/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin/cargo
WORKDIR /sandbox
USER sandbox
ENTRYPOINT ["sleep", "infinity"]
