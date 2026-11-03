# Writing tests

A test that needs files outside the repo must say so in its name, because CI selects tests by name:

- `needs_disc_…`: needs the disc image or the folder prepared from it.
- `needs_reference_…`: needs ctr-native at `reference/ctr-native`.
- `needs_disc_needs_reference_…`: needs both, in that order.

No other test or module may have `needs_disc` or `needs_reference` anywhere in its name.

CI runs with `--skip needs_disc` and sets `CTR_REQUIRE_REFERENCE`. If you leave the prefix off a disc test, CI still runs it, it finds no disc, skips its checks, and passes. If you leave it off a reference test, `--skip needs_reference` won't skip it on machines that don't have ctr-native.

Details are in `README.md`.
