package main

import (
	"flag"
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"

	"github.com/tanveerprottoy/logmux/aggregator/internal/pool"
	"github.com/tanveerprottoy/logmux/aggregator/internal/protocol"
	"github.com/tanveerprottoy/logmux/aggregator/internal/server"
)

func main() {
	// Parse CLI flags with systems defaults
	port := flag.Int("port", 8080, "TCP port to bind aggregator")
	workers := flag.Int("workers", 8, "Number of concurrent worker goroutines")
	queue := flag.Int("queue", 10000, "Worker pool queue capacity")
	rate := flag.Uint("rate", 10000, "Token bucket rate limit (events/sec per stream)")
	burst := flag.Uint("burst", 50000, "Token bucket burst capacity (events per stream)")
	flag.Parse()

	addr := fmt.Sprintf("0.0.0.0:%d", *port)
	fmt.Printf("[Aggregator] Starting LogMux Aggregator...\n")
	fmt.Printf("[Aggregator] Configuration: workers=%d, queue=%d, rate=%d/sec, burst=%d\n",
		*workers, *queue, *rate, *burst)

	batchPool := &sync.Pool{
		New: func() interface{} {
			return &protocol.LogEventBatch{
				Events: make([]protocol.LogEvent, 0, 100),
			}
		},
	}

	bufferPool := &sync.Pool{
		New: func() interface{} {
			return make([]byte, 64*1024)
		},
	}

	workerPool := pool.NewWorkerPool(*workers, *queue, batchPool, bufferPool)
	workerPool.Start()

	srv := server.NewTCPServer(addr, workerPool, batchPool, bufferPool, uint32(*rate), uint32(*burst))
	if err := srv.Start(); err != nil {
		fmt.Printf("[Critical] Failed to start TCPServer: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("[Aggregator] TCP listener successfully bound to %s\n", addr)

	// Graceful shutdown channel listening for termination signals
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

	sig := <-sigChan
	fmt.Printf("\n[Aggregator] Received signal %v, initiating graceful shutdown...\n", sig)

	// 1. Stop TCP server first to stop accepting new connection payloads
	srv.Stop()
	fmt.Printf("[Aggregator] TCP Server stopped.\n")

	// 2. Shut down the worker pool, allowing workers to drain outstanding channel items
	workerPool.Stop()
	fmt.Printf("[Aggregator] Worker Pool successfully drained and stopped.\n")

	fmt.Printf("[Aggregator] LogMux Aggregator clean shutdown complete.\n")
}
