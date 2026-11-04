#!/bin/sh
# Usage: web/build.sh [release|dev|dist] [webgl2|webgpu|both], into target/web. Needs
# wasm-bindgen-cli at Cargo.lock's version, wasm-opt for dist, and an LLVM clang that targets wasm
# (brew install llvm, or LLVM=<prefix>). The page draws through WebGPU where the browser has it and
# that build was made, and through WebGL2 elsewhere; with webgpu alone, a browser without WebGPU
# shows nothing.
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
backends="${2:-webgl2}"
case "$backends" in
webgl2 | webgpu) ;;
both) backends="webgl2 webgpu" ;;
*) echo "unknown backend: $backends (webgl2, webgpu or both)" >&2; exit 1 ;;
esac

rm -rf target/web/play
for backend in $backends; do
    if [ "$backend" = webgpu ]; then
        with=--features=webgpu
    else
        with=
    fi
    cargo build -p client --target wasm32-unknown-unknown --profile "$profile" $features $with
    wasm-bindgen --target web --no-typescript $names --out-dir target/web/play \
        --out-name "client-$backend" "target/wasm32-unknown-unknown/$dir/client.wasm"
    if [ "$profile" = dist ]; then
        wasm-opt -O3 "target/web/play/client-${backend}_bg.wasm" \
            -o "target/web/play/client-${backend}_bg.wasm"
    fi
done
cp web/index.html target/web/
# The page picks among the builds made here (`BUILT` in it).
built=$(printf '"%s", ' $backends)
sed "s/^const BUILT = \[\"webgl2\"\];/const BUILT = [${built%, }];/" web/play.html \
    >target/web/play/index.html
cp -R web/fonts target/web/
