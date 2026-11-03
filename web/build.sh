#!/bin/sh
# Usage: web/build.sh [release|dev|dist], into target/web. Needs wasm-bindgen-cli at Cargo.lock's
# version, wasm-opt for dist, and an LLVM clang that targets wasm (brew install llvm, or LLVM=<prefix>).
set -eu
cd "$(dirname "$0")/.."

if [ -n "${LLVM:-}" ]; then
    bin="$LLVM/bin/"
elif command -v brew >/dev/null && [ -d "$(brew --prefix llvm)" ]; then
    bin="$(brew --prefix llvm)/bin/"
else
    bin=
fi
export CC_wasm32_unknown_unknown="${bin}clang"
export AR_wasm32_unknown_unknown="${bin}llvm-ar"

profile="${1:-release}"
case "$profile" in
dev) dir=debug ;;
*) dir="$profile" ;;
esac

if [ "$profile" = dist ]; then
    features=--no-default-features
    names=--remove-name-section
else
    features=
    names=
fi
cargo build -p client --target wasm32-unknown-unknown --profile "$profile" $features
wasm-bindgen --target web --no-typescript $names --out-dir target/web/play --out-name client \
    "target/wasm32-unknown-unknown/$dir/client.wasm"
if [ "$profile" = dist ]; then
    wasm-opt -O3 target/web/play/client_bg.wasm -o target/web/play/client_bg.wasm
fi
cp web/index.html target/web/
cp web/play.html target/web/play/index.html
cp -R web/fonts target/web/
