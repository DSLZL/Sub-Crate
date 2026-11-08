#!/usr/bin/env bash
# Docker 29 defaults to the containerd image store, where image IDs are manifest
# digests. The release identity checks compare image IDs with OCI config digests.
set -euo pipefail
config=/etc/docker/daemon.json
current=$(sudo cat "$config" 2>/dev/null || echo '{}')
jq '.features["containerd-snapshotter"] = false' <<< "$current" | sudo tee "$config" > /dev/null
sudo systemctl restart docker
test "$(docker info --format '{{.Driver}}')" = overlay2
