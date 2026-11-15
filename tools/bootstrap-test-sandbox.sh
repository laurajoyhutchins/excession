#!/bin/sh
# Explicit one-time setup. No project source is built, mounted, or executed.
set -eu

if [ "$#" -ne 0 ]; then
  printf '%s\n' "Usage: sh tools/bootstrap-test-sandbox.sh" >&2
  exit 2
fi
if [ "$(uname -s)" != "Linux" ]; then
  printf '%s\n' "Excession's safe local test runner requires Linux rootless Podman." >&2
  exit 2
fi
if [ ! -x /usr/bin/podman ]; then
  printf '%s\n' "Install rootless Podman at /usr/bin/podman, then retry." >&2
  exit 2
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
image=localhost/excession-test-sandbox:latest

printf '%s\n' "Building a local Excession test image. This explicit setup uses the network to install pinned test tools."
printf '%s\n' "Only tools/sandbox/ is sent as the build context; your project source is not included."
env CONTAINERS_CONF=/dev/null CONTAINERS_CONF_OVERRIDE=/dev/null \
  CONTAINERS_MOUNTS_CONF=/dev/null TMPDIR=/tmp \
  /usr/bin/podman --remote=false build --pull=always \
  --tag "$image" \
  --file "$script_dir/sandbox/Containerfile" "$script_dir/sandbox"

image_id=$(env CONTAINERS_CONF=/dev/null CONTAINERS_CONF_OVERRIDE=/dev/null \
  CONTAINERS_MOUNTS_CONF=/dev/null TMPDIR=/tmp \
  /usr/bin/podman --remote=false image inspect --format '{{.Id}}' "$image")

image_id=${image_id#sha256:}
if ! printf '%s\n' "$image_id" | grep -Eq '^[0-9a-f]{64}$'; then
  printf 'Unexpected Podman image ID: %s\n' "$image_id" >&2
  exit 2
fi
image_id="sha256:$image_id"

printf '%s\n' "Installed image ID: $image_id"
printf '%s\n' "Next: excession tests --doctor"
printf '%s\n' "Then: excession tests; excession tests --run"
