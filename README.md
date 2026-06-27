# LogMux

A hybrid Rust/Zig and Go Zero-Copy Log Multiplexer — a lightweight, low-overhead alternative to Fluentd/FluentBit.

## Overview

LogMux consists of two primary components:

1. **Low-Level Collector (Rust/Zig)** – Runs as a Kubernetes DaemonSet, reading raw container log files from a Linux host natively using zero-copy slicing via inotify/epoll system calls.

2. **High-Scale Aggregator (Go)** – Runs inside a Kubernetes cluster, receiving multiplexed streams over a custom TCP/Protobuf protocol, handling concurrency with lock-free worker pools, and applying token-bucket rate limiting.

## Architecture

```
┌─────────────────────┐                      ┌──────────────────────┐
│   logmux-daemon     │  Persistent TCP    │   logmux-aggregator  │
│   (Rust/Zig)        │────────────────────▶   (Go, stdlib only)  │
│                     │                      │                      │
│ • inotify file tail │                      │ • Token Bucket RL      │
│ • Zero-copy parsing │                      │ • Worker pool          │
│ • epoll event loop  │                      │ • Metrics (pprof)      │
└─────────────────────┘                      └──────────────────────┘
         │                                               │
         ▼                                               ▼
   /var/log/pods/*                                stdout/stderr
   (Kubernetes CRI logs)                           (Application consumption)
```

## Directory Structure

```
logmux/
├── README.md                    # This file
├── Makefile                     # Unified build/test/lint commands
├── daemon/
│   └── rust/
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs            # Entry point, dispatcher, CRI path parsing
│       │   ├── collector/
│       │   │   ├── mod.rs
│       │   │   ├── sys.rs         # Linux syscall FFI (inotify, epoll, poll)
│       │   │   └── tailer.rs      # File tailing, inotify watch management
│       │   ├── parser/
│       │   │   ├── mod.rs         # LogParser trait, timestamp utils
│       │   │   ├── cri.rs         # CRI-format log parser
│       │   │   └── json.rs        # Docker JSON log parser
│       │   └── transport/
│       │       ├── mod.rs
│       │       ├── tcp.rs         # TcpClient with backoff & vectored writes
│       │       └── codec.rs       # Wire protocol framing
│       ├── build/
│       │   └── Dockerfile         # Multi-stage distroless image
│       └── charts/
│           └── logmux-daemon/     # Helm chart for Kubernetes
├── aggregator/
│   ├── go.mod
│   ├── cmd/
│   │   └── logmux-aggregator/
│   │       └── main.go            # Entry point, pool/init, graceful shutdown
│   ├── internal/
│   │   ├── server/
│   │   │   └── tcp.go             # TCP listener, connection handler
│   │   ├── pool/
│   │   │   └── worker.go            # Lock-free worker pool
│   │   ├── protocol/
│   │   │   └── codec.go             # Binary frame decoder (zero-copy)
│   │   └── ratelimit/
│   │       └── limiter.go           # Lock-free token bucket
│   ├── build/
│   │   └── Dockerfile             # Multi-stage distroless image
│   └── charts/
│       └── logmux-aggregator/     # Helm chart for Kubernetes
├── protocol/
│   └── protocol.md                # Wire protocol specification
├── infra/
│   ├── terraform/
│   │   ├── main.tf                # VPC, subnets, security groups
│   │   ├── variables.tf
│   │   └── outputs.tf
│   └── kubernetes/
│       ├── daemonset.yaml
│       ├── deployment.yaml
│       └── networkpolicy.yaml
└── .github/
    └── workflows/
        ├── ci.yml                 # Build, test, lint
        └── gitops.yml             # Deploy via ArgoCD/Flux
```

## Wire Protocol

See `protocol/protocol.md` for the complete binary framing specification:

- **Frame Header**: 5 bytes (4-byte BE length + 1-byte type)
- **Frame Types**: `0x01` (StreamRegister), `0x02` (LogEventBatch)
- **Zero-Copy**: Event payloads borrowed directly from read buffers

## Quick Start

```bash
# Build all components
make all

# Run tests
make test

# Lint both codebases
make lint
```

### Daemon

```bash
# Default: tails /var/log/pods, connects to 127.0.0.1:8080
LOGMUX_LOG_DIR=/var/log/pods LOGMUX_AGGREGATOR=127.0.0.1:8080 ./daemon/rust/target/release/logmux-daemon
```

### Aggregator

```bash
# Start with 8 workers, 10k queue, 10k events/sec rate limit
./aggregator/bin/logmux-aggregator -port 8080 -workers 8 -queue 10000 -rate 10000 -burst 50000
```

## Performance Characteristics

| Component | Target | Strategy |
|-----------|--------|----------|
| Memory | < 10MB RSS | `sync.Pool`, pre-allocated buffers, zero-copy slicing |
| Latency | < 1ms | epoll/kqueue event loop, lock-free rate limiting |
| Throughput | 100k+ events/sec | Batched sends, vectored writes, worker parallelism |

## Phase 1 Implementation Path (Low-Level Collector)

### Step 1: Syscall Validation
- [x] `sys.rs`: inotify_init1, poll, read, close FFI bindings
- [x] inotify_add_watch, inotify_rm_watch bindings
- [ ] Add epoll support for future io_uring migration

### Step 2: File Tailer Core
- [x] `tailer.rs`: Recursive directory scanning, `.log` file detection
- [x] `tailer.rs`: inotify watch management (create/move/delete)
- [ ] Handle file rotation edge cases (inode reuse, truncate detection)

### Step 3: Zero-Copy Parsing
- [x] `cri.rs`: CRI format parser (timestamp, stream, tag, payload)
- [x] `json.rs`: Docker JSON log parser with escaped sequence handling
- [x] `mod.rs`: RFC3339 timestamp parser to nanoseconds
- [ ] Add `std::borrow::Cow` support for partial vs complete lines

### Step 4: Transport Layer
- [x] `tcp.rs`: TcpClient with exponential backoff (250ms → 30s)
- [x] `codec.rs`: Custom wire protocol encoding with IoSlices
- [ ] Add connection health heartbeat

### Step 5: Integration Testing
- [ ] Create integration test with mock aggregator
- [ ] Benchmark: 1K/10K/100K concurrent log streams
- [ ] Fuzz CRI parser with malformed input

## License

Apache 2.0