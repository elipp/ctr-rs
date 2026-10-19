---
name: project-scope-online-cups
description: ctr-rs scope is physics-accurate online multiplayer cups; Adventure Mode is out of scope
metadata:
  node_type: memory
  type: project
  originSessionId: d768b460-1f81-4f93-bfbb-67dcf600f8b1
  modified: 2026-10-18T15:01:24.384Z
---

The Rust port focuses on physics-accurate online multiplayer cups. Anything related to Adventure Mode (hub levels, adventure packs, garage, boss/podium flows, etc.) can be left out.

**Why:** User stated the port's goal explicitly (2026-10-18).
**How to apply:** When porting from [[ctr-native-reference]], skip Adventure-only code paths and assets. Prioritize race tracks, racer physics/determinism, and netplay-relevant systems. Don't add speculative support for adventure content.
