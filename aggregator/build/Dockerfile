# syntax=docker/dockerfile:1.7
FROM golang:1.24-alpine AS builder

WORKDIR /src
COPY aggregator/go.mod aggregator/go.sum* /

RUN go mod download || true

COPY aggregator/cmd /src/cmd
COPY aggregator/internal /src/internal

RUN CGO_ENABLED=0 go build -o /logmux-aggregator ./cmd/logmux-aggregator

FROM gcr.io/distroless/static-debian12
COPY --from=builder /logmux-aggregator /logmux-aggregator

USER nonroot:nonroot
ENTRYPOINT ["/logmux-aggregator"]