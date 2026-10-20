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

**Skid marks (2026-10-20, NOT visually verified):** `vehicle::skid` (`VehEmitter_Skidmarks` ring of 8 ticks x 4 tires, plus the segment walk of `VehGroundSkids_Main`), `view::skid` (two meshes: dark = subtractive, light = quarter-add for `FORCE_SKIDMARKS` terrain; `tread` icon, global index 0x2f). Known differences:
- The marks' OT depth bias bytes (`depthBiasNormal/Secondary`) are not stored; the marks are lifted 1 game unit and get a material `depth_bias` of 4 instead.
- Brightness follows retail: one value per driver from the depth of the newest mark (full inside 0x180 units, halving per doubling beyond, gone below 0x10), halved per tick of age. We take the depth along the Bevy camera's forward axis in place of the GTE view matrix's z, and skip the per-vertex "depth > 0x20" test.
- Blend strength: the maths is retail's (dark = background - texel, light = background + texel/4), but Bevy blends in linear light and the PS1 in raw values, so the marks read lighter. `DARK_GAIN` / `LIGHT_GAIN` in `view/src/skid.rs` are tuned by eye.
- The wheelie now changes which tires skid, as in `VehEmitter_DriverMain`.
- The strips are at tick resolution; the kart is interpolated between ticks, the marks are not.
- Only the player's kart (`--drive`) leaves marks.

**Wheelie (2026-10-20, NOT visually verified):** `vehicle::baked` (the authored `matArr01..03` of `data.bakedGteMath`, `VehPhysForce_TranslateMatrix_UpdateMatrixAnimation`, and `inst_matrix` = `UpdateInstanceMatrix`), `vehicle::rotmat` (`ConvertRotToMatrix` moved from `camera`, plus `MatrixRotate`). `view::drive::pose_of` takes the kart's pose from `inst_matrix`. The exhaust pipes, skid mark tires and the flames follow it. Known differences:
- Only the wheelie tables are ported. For crash fall, squish recovery, blasted and the airborne jump poses `inst_matrix` is still the facing matrix. (`frame.rs` already sets those `matrix_array`s; they just have no tables yet.)
- The squash/shear of the baked matrix is approximated on the Bevy side: rotation from the (re-orthogonalised) up and forward columns, scale from the column lengths.
- The squish offset (`squishTimer`) and the instance scale squash/stretch (`inst->scale`) are not ported, as before.
- The oracle tests (`tick.rs`, `reference.rs`) now see `matrixArray` 1..3 during boosts; they need a run on Linux.
