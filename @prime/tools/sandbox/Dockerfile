FROM rust:1.90.0-bookworm AS polaris-guest-build
WORKDIR /workspace
COPY . /workspace
RUN cargo build --locked --release -p polaris-guest --bin polaris-guest

ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates coreutils \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 1000 sandbox \
    && install -d -o sandbox -g sandbox /sandbox
COPY --from=polaris-guest-build /workspace/target/release/polaris-guest /usr/bin/polaris-guest
RUN chmod 0755 /usr/bin/polaris-guest
WORKDIR /sandbox
USER sandbox
ENTRYPOINT ["sleep", "infinity"]
