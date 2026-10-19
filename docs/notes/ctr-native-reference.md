---
name: ctr-native-reference
description: The C reference implementation of CTR (ctr-native) lives in ./reference/ctr-native in the ctr-rs repo
metadata:
  node_type: memory
  type: reference
  originSessionId: d768b460-1f81-4f93-bfbb-67dcf600f8b1
  modified: 2026-10-18T14:54:02.148Z
---

ctr-rs is a Rust rewrite of Crash Team Racing. The C reference (ctr-native, decompilation-based) is at `/home/elias/devel/ctr-rs/reference/ctr-native` (untracked in git). Look there for game logic, e.g. `game/LOAD/LOAD_File.c`, `LOAD_Queue.c`, `LOAD_Assets.c`, `CDSYS.c` for BIGFILE.BIG handling.

**Why:** User pointed it out as the source of truth to port from.
**How to apply:** Consult it before implementing any subsystem (BIGFILE, level loading, etc.).
