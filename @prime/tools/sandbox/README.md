# Polaris OpenShell sandbox image

Build this local image from WSL2 at the repository root with `bash @prime/tools/sandbox/build-wsl.sh`. The script resolves Ubuntu 24.04 to its current immutable OCI digest, builds the pinned workspace guest binary, then prints the local Docker image content ID and checks `/usr/bin/polaris-guest --protocol-version` inside the image. The output image tag is `polaris-sandbox:p2` unless `POLARIS_SANDBOX_TAG` is set. The script does not push the image.

Use the printed `image_id` (`sha256:<64 hex>`) in the blueprint's `components.sandbox.image`; the blueprint digest then binds that immutable local image reference.
