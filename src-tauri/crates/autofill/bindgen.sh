#!/bin/sh
# Regenerates the Swift bindings the iOS AutoFill extension compiles against,
# into src-tauri/gen/apple/Sources/autofill/generated/, which is committed:
# the Swift, the C header it calls the library through, and the module map
# the extension imports that header by (OTHER_SWIFT_FLAGS in project.yml).
#
# Re-run it, and commit what it writes, whenever this crate's exported API
# changes — anything `#[uniffi::export]`ed or derived as a `uniffi::Record`,
# `uniffi::Object` or `uniffi::Error` — or the `uniffi` version does. The
# bindings check the library they are linked against on load, so a stale copy
# fails at the extension's first call rather than quietly misbehaving.
#
# UniFFI's library mode: the API is read from the built library itself, by the
# `uniffi-bindgen` binary in this crate (the version the runtime is). Built for
# the host — the bindings are the same for every target. Honours
# CARGO_TARGET_DIR like any cargo command.
set -eu

cd "$(dirname "$0")/../.."
target_dir="${CARGO_TARGET_DIR:-target}"
out=gen/apple/Sources/autofill/generated

cargo build -p rowel-autofill
cargo run -q -p rowel-autofill --features bindgen --bin uniffi-bindgen -- \
  generate --library "$target_dir/debug/librowel_autofill.a" \
  --language swift --out-dir "$out"
