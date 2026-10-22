#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BASE_REPOSITORY="ubuntu:24.04"
IMAGE_TAG="${POLARIS_SANDBOX_TAG:-polaris-sandbox:p2}"

docker pull "$BASE_REPOSITORY"
BASE_IMAGE="$(docker image inspect "$BASE_REPOSITORY" --format '{{index .RepoDigests 0}}')"
if [[ -z "$BASE_IMAGE" || "$BASE_IMAGE" == "<no value>" ]]; then
  printf 'could not resolve immutable digest for %s\n' "$BASE_REPOSITORY" >&2
  exit 1
fi

docker build \
  --file "$ROOT/@prime/tools/sandbox/Dockerfile" \
  --build-arg "BASE_IMAGE=$BASE_IMAGE" \
  --tag "$IMAGE_TAG" \
  "$ROOT"

IMAGE_ID="$(docker image inspect "$IMAGE_TAG" --format '{{.Id}}')"
if [[ ! "$IMAGE_ID" =~ ^sha256:[0-9a-f]{64}$ ]]; then
  printf 'unexpected local image ID: %s\n' "$IMAGE_ID" >&2
  exit 1
fi
printf 'tag=%s\nimage_id=%s\nbase_image=%s\n' "$IMAGE_TAG" "$IMAGE_ID" "$BASE_IMAGE"
docker run --rm --entrypoint /usr/bin/polaris-guest "$IMAGE_ID" --protocol-version
