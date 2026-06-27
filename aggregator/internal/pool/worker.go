package pool

import (
	"fmt"
	"sync"
	"time"

	"github.com/tanveerprottoy/logmux/aggregator/internal/protocol"
)

// FrameJob pairs a decoded batch with the read buffer that owns event payload bytes.
type FrameJob struct {
	Batch *protocol.LogEventBatch
	Buf   []byte
}

type WorkerPool struct {
	workChan   chan FrameJob
	numWorkers int
	batchPool  *sync.Pool
	bufferPool *sync.Pool
	wg         sync.WaitGroup
}

// NewWorkerPool creates a new worker pool instance.
func NewWorkerPool(numWorkers int, queueSize int, batchPool, bufferPool *sync.Pool) *WorkerPool {
	return &WorkerPool{
		workChan:   make(chan FrameJob, queueSize),
		numWorkers: numWorkers,
		batchPool:  batchPool,
		bufferPool: bufferPool,
	}
}

// Start spawns the configured number of worker goroutines.
func (wp *WorkerPool) Start() {
	for i := 0; i < wp.numWorkers; i++ {
		wp.wg.Add(1)
		go wp.workerLoop()
	}
}

// Submit sends a batch and its backing buffer to the worker queue.
func (wp *WorkerPool) Submit(job FrameJob) {
	wp.workChan <- job
}

// Stop gracefully closes the work channel and waits for all workers to finish.
func (wp *WorkerPool) Stop() {
	close(wp.workChan)
	wp.wg.Wait()
}

func (wp *WorkerPool) workerLoop() {
	defer wp.wg.Done()

	for job := range wp.workChan {
		batch := job.Batch
		for _, event := range batch.Events {
			t := time.Unix(0, event.TimestampNS).UTC()
			streamStr := "stdout"
			if event.IsStderr {
				streamStr = "stderr"
			}

			fmt.Printf("[%s] StreamID: %d | Time: %s | Msg: %s\n",
				streamStr,
				batch.StreamID,
				t.Format("15:04:05.000000"),
				string(event.Payload),
			)
		}

		batch.Reset()
		wp.batchPool.Put(batch)
		wp.bufferPool.Put(job.Buf)
	}
}
