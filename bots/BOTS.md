The current default "brain" was trained with:
	cargo run --release -p brain --bin train -- out=target/switch-v4 gamma=0.995 drift_boost=11 boost_growth=2 steer_change=0.3 weights=11:3,12:3,13:3,5:2,7:2,10:2 iters=1000
