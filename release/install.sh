#!/bin/sh
set -eu

case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) archive=gov-x86_64-unknown-linux-musl.tar.gz ;;
    *) echo "This installer supports Linux x86-64." >&2; exit 1 ;;
esac

base=https://github.com/SinVC/comfy_env/releases/download/@RELEASE_TAG@
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

curl --proto '=https' -fsSL "$base/$archive" -o "$stage/$archive"
curl --proto '=https' -fsSL "$base/$archive.sha256" -o "$stage/$archive.sha256"
expected=$(tr -d '\r\n' < "$stage/$archive.sha256")
actual=$(sha256sum "$stage/$archive")
if [ "${actual%% *}" != "$expected" ]; then
    echo "SHA-256 mismatch for $archive" >&2
    exit 1
fi

tar -xzf "$stage/$archive" -C "$stage"
"$stage/gov" setup "$@"
