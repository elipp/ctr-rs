---
name: target-frame-rate-30hz
description: ctr-rs targets multiples of 30 FPS; game logic and animation run on a fixed 30 Hz tick
metadata:
  type: project
---

The game tick is 30 Hz (the original `gGT->timer` increments once per 30 Hz frame) and the port will run at multiples of 30 FPS.

**Why:** user said so on 2026-10-18 while texture animation (AnimTex cycling) was being planned.
**How to apply:** anything frame-dependent (texture cycling, physics, timers) must be driven by an integer 30 Hz tick counter, never by render frames or wall-clock delta directly; the renderer may interpolate between ticks. Related: [[project-status-collision-plan]].
