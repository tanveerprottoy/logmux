package server

import (
	"fmt"
	"io"
	"net"
	"sync"
	"sync/atomic"

	"github.com/tanveerprottoy/logmux/aggregator/internal/pool"
	"github.com/tanveerprottoy/logmux/aggregator/internal/protocol"
	"github.com/tanveerprottoy/logmux/aggregator/internal/ratelimit"
)

type TCPServer struct {
	addr       string
	workerPool *pool.WorkerPool
	batchPool  *sync.Pool
	bufferPool *sync.Pool
	listener   net.Listener
	closed     int32
	wg         sync.WaitGroup

	rate  uint32
	burst uint32
}

// NewTCPServer initializes a raw TCP server with pooled buffers and rate limits.
func NewTCPServer(addr string, workerPool *pool.WorkerPool, batchPool, bufferPool *sync.Pool, rate, burst uint32) *TCPServer {
	return &TCPServer{
		addr:       addr,
		workerPool: workerPool,
		batchPool:  batchPool,
		bufferPool: bufferPool,
		rate:       rate,
		burst:      burst,
	}
}

// Start opens the listener socket and starts the connection accept loop.
func (s *TCPServer) Start() error {
	l, err := net.Listen("tcp", s.addr)
	if err != nil {
		return err
	}
	s.listener = l

	s.wg.Add(1)
	go s.acceptLoop()

	return nil
}

// Stop closes the listener socket and awaits completion of active connection routines.
func (s *TCPServer) Stop() {
	if !atomic.CompareAndSwapInt32(&s.closed, 0, 1) {
		return
	}
	s.listener.Close()
	s.wg.Wait()
}

func (s *TCPServer) acceptLoop() {
	defer s.wg.Done()

	for {
		conn, err := s.listener.Accept()
		if err != nil {
			if atomic.LoadInt32(&s.closed) != 0 {
				break
			}
			fmt.Printf("[TCPServer] Accept error: %v\n", err)
			continue
		}

		s.wg.Add(1)
		go s.handleConnection(conn)
	}
}

func (s *TCPServer) handleConnection(conn net.Conn) {
	defer s.wg.Done()
	defer conn.Close()

	buf := s.bufferPool.Get().([]byte)
	defer s.bufferPool.Put(buf)

	limiters := make(map[uint32]*ratelimit.TokenBucket)
	streams := make(map[uint32]*protocol.StreamRegister)

	for {
		pooledBatch := s.batchPool.Get().(*protocol.LogEventBatch)
		frameType, frame, err := protocol.ReadFrame(conn, buf, pooledBatch)
		if err != nil {
			s.batchPool.Put(pooledBatch)
			if err != io.EOF && atomic.LoadInt32(&s.closed) == 0 {
				fmt.Printf("[TCPServer] Error reading from client %s: %v\n", conn.RemoteAddr(), err)
			}
			break
		}

		switch frameType {
		case protocol.FrameStreamRegister:
			s.batchPool.Put(pooledBatch)

			reg := frame.(*protocol.StreamRegister)
			streams[reg.StreamID] = reg
			limiters[reg.StreamID] = ratelimit.NewTokenBucket(s.rate, s.burst)

			fmt.Printf("[TCPServer] Registered Stream %d | Namespace: %s | Pod: %s | Container: %s\n",
				reg.StreamID, reg.Namespace, reg.PodName, reg.ContainerName)

		case protocol.FrameLogEventBatch:
			batch := frame.(*protocol.LogEventBatch)

			limiter, ok := limiters[batch.StreamID]
			if !ok {
				limiter = ratelimit.NewTokenBucket(s.rate, s.burst)
				limiters[batch.StreamID] = limiter
			}

			eventCount := uint32(len(batch.Events))
			if !limiter.Allow(eventCount) {
				batch.Reset()
				s.batchPool.Put(batch)
				continue
			}

			submittedBuf := buf
			buf = s.bufferPool.Get().([]byte)
			s.workerPool.Submit(pool.FrameJob{Batch: batch, Buf: submittedBuf})
		}
	}
}
