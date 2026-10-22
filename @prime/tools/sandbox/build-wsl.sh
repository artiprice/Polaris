#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BASE_REPOSITORY="nvcr.io/nvidia/base/ubuntu:24.04"
IMAGE_TAG="${POLARIS_SANDBOX_TAG:-polaris-sandbox:p2}"

docker pull "$BASE_REPOSITORY"
BASE_IMAGE="$(docker image inspect "$BASE_REPOSITORY" --format '{{index .RepoDigests 0}}')"
if [[ -z "$BASE_IMAGE" || "$BASE_IMAGE" == "<no value>" ]]; then
  printf 'could not resolve immutable digest for %s\n' "$BASE_REPOSITORY" >&2
  exit 1
fi

CONTEXT_DIR="$(mktemp -d /tmp/polaris-sandbox-context.XXXXXX)"
cleanup_context() {
  case "$CONTEXT_DIR" in
    /tmp/polaris-sandbox-context.*) rm -rf -- "$CONTEXT_DIR" ;;
    *) printf 'refusing to remove unexpected build context: %s\n' "$CONTEXT_DIR" >&2 ;;
  esac
}
trap cleanup_context EXIT

cp "$ROOT/@prime/tools/sandbox/Dockerfile" "$CONTEXT_DIR/Dockerfile"
tar \
  --directory "$ROOT" \
  --exclude=target \
  --exclude=*/target \
  --exclude=node_modules \
  --exclude=*/node_modules \
  --exclude=dist \
  --exclude=*/dist \
  --exclude=tests/fuzz \
  --exclude=tests/fixtures \
  --exclude=tests/spike/*/tests \
  --exclude=tests/tests \
  --create --file - \
  Cargo.toml Cargo.lock rust-toolchain.toml .cargo apps crates tests xtask vendor/biscuit-auth \
  | tar --directory "$CONTEXT_DIR" --extract --file -

if find "$CONTEXT_DIR" -type f \
  \( -name '.env' -o -name '.env.*' -o -name 'credentials' -o -name 'credentials.toml' \
     -o -name '.netrc' -o -name '*.pem' -o -name '*.key' \) \
  -print -quit | grep -q .; then
  printf 'refusing Docker build context containing credentials or private keys\n' >&2
  exit 1
fi

if [[ -e "$CONTEXT_DIR/.aws" || -e "$CONTEXT_DIR/.ssh" || -e "$CONTEXT_DIR/.codex" ]]; then
  printf 'refusing Docker build context containing machine credential directories\n' >&2
  exit 1
fi

docker build \
  --file "$CONTEXT_DIR/Dockerfile" \
  --build-arg "BASE_IMAGE=$BASE_IMAGE" \
  --tag "$IMAGE_TAG" \
  "$CONTEXT_DIR"

IMAGE_ID="$(docker image inspect "$IMAGE_TAG" --format '{{.Id}}')"
if [[ ! "$IMAGE_ID" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  printf 'unexpected local image ID: %s\n' "$IMAGE_ID" >&2
  exit 1
fi
printf 'tag=%s\nimage_id=%s\nbase_image=%s\n' "$IMAGE_TAG" "$IMAGE_ID" "$BASE_IMAGE"
docker run --rm --entrypoint /usr/bin/polaris-guest "$IMAGE_ID" --protocol-version
docker run --rm --entrypoint bash "$IMAGE_ID" -lc 'set -eu; test -x /usr/bin/polaris-guest; test ! -e "$CARGO_HOME/credentials.toml"; test -x "$RUSTUP_HOME/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin/rustc"; rustc --version; cargo --version; rustup toolchain list; test "$RUSTUP_AUTO_INSTALL" = 0'
