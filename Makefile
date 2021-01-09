
.PHONY = release build test clean

release:
	cargo build --release
	cp ./target/release/myfind myfind

build:
	cargo build

test:
	cargo test
	cargo build --release
	./testing.sh

clean:
	cargo clean
