The current default "brain" (`policy/no-chain-boost-v1.safetensors`, checkpoint 1000 of the run `air-steer-retry`) was trained with:
	cargo run --release -p brain --bin train -- out=training-output/air-steer-retry gamma=0.995 boost_growth=2.0 drift_boost=5 boost_fade=100,500 boost_timed=12 steer_change=0.5 iters=1000 weights=11:3,12:3,13:3,5:2,7:2,10:2

It drives faster than the scripted bot on all 18 tracks (path 0), but rarely chains drift boosts.
