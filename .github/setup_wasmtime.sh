#!/bin/sh
set -e

if test -z "$RUNNER_OS"; then
    echo "Should only run in CI!"
    exit 1
fi

url=https://github.com/bytecodealliance/wasmtime/releases/download/v42.0.1/wasmtime-v42.0.1-x86_64-linux.tar.xz

cd /tmp
curl -L "$url" | tar Jx
mv wasmtime-*/wasmtime .

mkdir -p ~/.cargo
>> ~/.cargo/config.toml cat <<'EOF'
[target.'cfg(target_os = "wasi")']
runner = "/tmp/wasmtime"
EOF
