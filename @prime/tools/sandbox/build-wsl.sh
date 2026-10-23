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

python3 "$ROOT/@prime/tools/sandbox/build-context.py" "$ROOT" "$CONTEXT_DIR"
cp "$ROOT/@prime/tools/sandbox/Dockerfile" "$CONTEXT_DIR/Dockerfile"

BUILD_CONTEXT_HASH="$(
  tar \
    --directory "$CONTEXT_DIR" \
    --sort=name \
    --mtime='UTC 1970-01-01' \
    --owner=0 --group=0 --numeric-owner \
    --create --file - . \
    | sha256sum | cut -d' ' -f1
)"
printf 'build_context_sha256=%s\n' "$BUILD_CONTEXT_HASH"

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
