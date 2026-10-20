**Ground shadow (2026-10-20, uncommitted, NOT visually verified):** `vehicle::shadow` (cull + orientation), `render::shadow` (4 quads, atlas of global icons 0/1 `shadow1`/`shadow2`), `view::shadow` (subtractive mesh, `--drive` only). Known differences from `VehGroundShadow_Main`:
- Camera-distance fade: retail only fades in the "small screen" case (the push buffer's screen-distance value is <= 0x100, which I read as split-screen). One player uses the fixed colour 0x1f1f1f, and so do we. The fade is not ported.
- Texel blending: every visible texel is subtracted. Retail forces the tpage to subtractive, so only texels with the semi-transparency (STP) bit blend and the rest draw opaque. Not checked whether any shadow texels lack STP.
- Depth: relies on the 3-unit lift above the ground plus the depth test. On a steep or bumpy surface the flat shadow can dip into the ground. If it flickers or gets cut off, add a material depth bias.
- Standing kart: the non-driving view (kart on the grid) has no shadow yet.
- No ordering-table depth bias (`depthBiasNormal` / `depthBiasSecondary`). Bevy's depth test stands in for it.

