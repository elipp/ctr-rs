---
name: dev-environment-c-oracles
description: The user's Mac cannot build/run the gcc -m32 C reference oracles; they run those tests on a Linux machine
metadata:
  type: user
---

On the user's macOS arm64 machine `gcc -m32` cannot link (clang fake-gcc, arm64 SDK), so the differential tests that build the C reference (`crates/*/tests/reference*.rs`, `tick.rs`) cannot run here. They skip silently anyway because `support::reference_dir()` expects `reference/ctr-native` which does not exist in this checkout (reference/ is the ctr-native tree itself).

**Why:** User said so on 2026-10-19 ("I'll transfer to a Linux machine").
**How to apply:** Don't try to run C oracles on the Mac; write them carefully, syntax-check with `clang -fsyntax-only -DCTR_NATIVE -w -I reference/include -I reference/game -I reference` (ignore the 64-bit static asserts), keep verifying with Rust-only tests (`cargo test -p vehicle --profile=dev --test drive`), and tell the user what to run on Linux. Table tests can be run here by temporarily symlinking `reference/ctr-native -> .` (remove afterwards). Related: [[ctr-native-reference]], [[project-status-collision-plan]].
