**Ground shadow (2026-10-20, uncommitted, NOT visually verified):** `vehicle::shadow` (cull + orientation), `render::shadow` (4 quads, atlas of global icons 0/1 `shadow1`/`shadow2`), `view::shadow` (subtractive mesh, `--drive` only). Known differences from `VehGroundShadow_Main`:
- Camera-distance fade: retail only fades in the "small screen" case (the push buffer's screen-distance value is <= 0x100, which I read as split-screen). One player uses the fixed colour 0x1f1f1f, and so do we. The fade is not ported.
- Texel blending: every visible texel is subtracted. Retail forces the tpage to subtractive, so only texels with the semi-transparency (STP) bit blend and the rest draw opaque. Not checked whether any shadow texels lack STP.
- Depth: relies on the 3-unit lift above the ground plus the depth test. On a steep or bumpy surface the flat shadow can dip into the ground. If it flickers or gets cut off, add a material depth bias.
- Standing kart: the non-driving view (kart on the grid) has no shadow yet.
- No ordering-table depth bias (`depthBiasNormal` / `depthBiasSecondary`). Bevy's depth test stands in for it.


**Exhaust fire (2026-10-20, NOT visually verified):** `vehicle::turbo` (state + `VehTurbo_ThTick`), `fire::increment` lights/restarts it, `view::flame` (8 models `turbo`..`turbo7` from the racer pack, per pipe, second mirrored and 3 frames ahead). Known differences:
- Depth fade: retail's `DEPTH_FADE` hides the fire when `alphaScale + 2*viewDepth > 0x1000`, and fades vertices near the camera. We hide it when `alphaScale >= 0x800` (a guess at a typical chase distance) and do no vertex fade.
- `DRAW_BILLBOARD` is not consumed anywhere in the reference, so the fire just follows the kart's orientation.
- Not modelled: sound, rumble, the driver's alpha halving while the fire burns, water split/reflection flags.
- `exhaust` still uses `reserves > 0` instead of retail's `PROC_SearchForModel` for "the flame exists".
