.PHONY: all build-daemon build-aggregator test clean lint

all: build-daemon build-aggregator

build-daemon:
	@echo "Building Rust daemon (debug)..."
	cd daemon/rust && cargo build

build-daemon-release:
	@echo "Building Rust daemon (release)..."
	cd daemon/rust && cargo build --release

build-aggregator:
	@echo "Building Go aggregator..."
	mkdir -p aggregator/bin
	cd aggregator && go build -o bin/logmux-aggregator cmd/logmux-aggregator/main.go

test:
	@echo "Running Rust daemon tests..."
	cd daemon/rust && cargo test
	@echo "Running Go aggregator tests..."
	cd aggregator && go test -v ./...

lint:
	@echo "Linting Rust code..."
	cd daemon/rust && cargo clippy --workspace --all-targets -- -D warnings
	cd daemon/rust && cargo fmt --all -- --check
	@echo "Linting Go code..."
	cd aggregator && go vet ./...

clean:
	@echo "Cleaning build directories..."
	cd daemon/rust && cargo clean
	rm -rf aggregator/bin

