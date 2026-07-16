#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
key_dir="$script_dir/ssh"
key_path="$key_dir/openraft-jepsen"

mkdir -p "$key_dir"
chmod 700 "$key_dir"

if [ -f "$key_path" ]; then
  if [ ! -f "$key_path.pub" ]; then
    ssh-keygen -y -f "$key_path" > "$key_path.pub"
  fi

  chmod 600 "$key_path"
  chmod 644 "$key_path.pub"
  echo "Jepsen SSH key already exists: $key_path"
  exit 0
fi

if [ -f "$key_path.pub" ]; then
  echo "Refusing to overwrite $key_path.pub without $key_path" >&2
  exit 1
fi

ssh-keygen \
  -t ed25519 \
  -N "" \
  -C "openraft-jepsen-docker" \
  -f "$key_path"

chmod 600 "$key_path"
chmod 644 "$key_path.pub"

echo "Generated Jepsen SSH key: $key_path"
