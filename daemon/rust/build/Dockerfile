# syntax=docker/dockerfile:1.7
FROM rust:1.85-alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /src
COPY Cargo.toml .
COPY src ./src

RUN cargo build --release

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /src/target/release/logmux-daemon /logmux-daemon

USER nonroot:nonroot
ENTRYPOINT ["/logmux-daemon"]