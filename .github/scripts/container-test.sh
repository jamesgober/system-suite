#!/usr/bin/env bash
# Runs the example service in a container with a memory and CPU limit, as a
# user with no entry in the user database: it boots, stops cleanly on
# SIGTERM, and the memory watermark reads the container's limit, not the
# host's. Every container is removed, even when the test fails.
set -euo pipefail

binary="$PWD/target/release/examples/service"
work="$(mktemp -d)"
config="$work/service.conf"
printf '[kernel]\nmemory_watch = "200ms"\nmemory_shed = 80\n' > "$config"
chmod 640 "$config"

name="system-suite-limits-$$"
cleanup() {
    docker rm -f "$name" > /dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

# start <fill in MiB>
start() {
    docker run -d --name "$name" \
        --memory=256m --memory-swap=256m --cpus=1 \
        --user "$(id -u):$(id -g)" \
        -e HOME=/tmp \
        -e SERVICE_EXAMPLE_CONFIG=/srv/service.conf \
        -e SERVICE_FILL_MB="$1" \
        -v "$binary:/srv/service:ro" \
        -v "$config:/srv/service.conf:ro" \
        ubuntu:24.04 /srv/service > /dev/null
}

# wait_for <text>
wait_for() {
    for _ in $(seq 1 150); do
        if docker logs "$name" 2>&1 | grep -q "$1"; then
            return 0
        fi
        if [ "$(docker inspect --format '{{.State.Running}}' "$name")" != "true" ]; then
            break
        fi
        sleep 0.2
    done
    echo "'$1' never appeared:"
    docker logs "$name" 2>&1
    return 1
}

# stop_cleanly: SIGTERM, then exit code 0, within the default deadline.
stop_cleanly() {
    docker stop --time 30 "$name" > /dev/null
    code="$(docker inspect --format '{{.State.ExitCode}}' "$name")"
    if [ "$code" != "0" ]; then
        echo "exit code $code, expected 0:"
        docker logs "$name" 2>&1
        return 1
    fi
    docker rm "$name" > /dev/null
}

echo "== a quiet service stays at normal"
start 0
wait_for "ready"
sleep 2
if docker logs "$name" 2>&1 | grep -q "urgent level: shed"; then
    echo "shed with nothing using memory:"
    docker logs "$name" 2>&1
    exit 1
fi
stop_cleanly

echo "== 225 MiB of a 256 MiB limit raises the line to shed"
start 225
wait_for "urgent level: shed"
stop_cleanly

echo "container test passed"
