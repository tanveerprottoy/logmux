package protocol

import (
	"bytes"
	"encoding/binary"
	"io"
	"testing"
	"unsafe"
)

func TestReadFrameStreamRegister(t *testing.T) {
	// Build a raw binary StreamRegister frame
	// Namespace: "default", Pod: "nginx-123", UID: "uid-abc", Container: "web"
	ns := "default"
	pod := "nginx-123"
	uid := "uid-abc"
	container := "web"

	payloadLen := 4 + 2 + len(ns) + 2 + len(pod) + 2 + len(uid) + 2 + len(container)

	buf := new(bytes.Buffer)
	// Write Header: Length (4 bytes), Type (1 byte)
	binary.Write(buf, binary.BigEndian, uint32(payloadLen))
	binary.Write(buf, binary.BigEndian, byte(FrameStreamRegister))

	// Write Payload
	binary.Write(buf, binary.BigEndian, uint32(42)) // stream_id
	binary.Write(buf, binary.BigEndian, uint16(len(ns)))
	buf.WriteString(ns)
	binary.Write(buf, binary.BigEndian, uint16(len(pod)))
	buf.WriteString(pod)
	binary.Write(buf, binary.BigEndian, uint16(len(uid)))
	buf.WriteString(uid)
	binary.Write(buf, binary.BigEndian, uint16(len(container)))
	buf.WriteString(container)

	destBuf := make([]byte, 1024)
	frameType, frame, err := ReadFrame(buf, destBuf, nil)
	if err != nil {
		t.Fatalf("Failed to read stream register frame: %v", err)
	}

	if frameType != FrameStreamRegister {
		t.Errorf("Expected frame type 0x01, got 0x%02x", frameType)
	}

	reg, ok := frame.(*StreamRegister)
	if !ok {
		t.Fatalf("Expected *StreamRegister type, got %T", frame)
	}

	if reg.StreamID != 42 {
		t.Errorf("Expected StreamID 42, got %d", reg.StreamID)
	}
	if reg.Namespace != ns || reg.PodName != pod || reg.PodUID != uid || reg.ContainerName != container {
		t.Errorf("Decoded metadata mismatch: %+v", reg)
	}
}

func TestReadFrameLogEventBatch(t *testing.T) {
	// Build a raw LogEventBatch frame with 2 events
	streamID := uint32(99)
	eventCount := uint32(2)

	log1 := []byte("log event number 1")
	log2 := []byte("error: connection reset by peer")

	// Payload layout:
	// stream_id (4b) + event_count (4b)
	// Event 1: timestamp (8b) + is_stderr (1b) + payload_len (4b) + payload (len1)
	// Event 2: timestamp (8b) + is_stderr (1b) + payload_len (4b) + payload (len2)
	payloadLen := 4 + 4 + (8 + 1 + 4 + len(log1)) + (8 + 1 + 4 + len(log2))

	buf := new(bytes.Buffer)
	// Write Header
	binary.Write(buf, binary.BigEndian, uint32(payloadLen))
	binary.Write(buf, binary.BigEndian, byte(FrameLogEventBatch))

	// Write Batch metadata
	binary.Write(buf, binary.BigEndian, streamID)
	binary.Write(buf, binary.BigEndian, eventCount)

	// Event 1 (stdout)
	binary.Write(buf, binary.BigEndian, int64(1000000001))
	binary.Write(buf, binary.BigEndian, byte(0)) // stdout
	binary.Write(buf, binary.BigEndian, uint32(len(log1)))
	buf.Write(log1)

	// Event 2 (stderr)
	binary.Write(buf, binary.BigEndian, int64(1000000002))
	binary.Write(buf, binary.BigEndian, byte(1)) // stderr
	binary.Write(buf, binary.BigEndian, uint32(len(log2)))
	buf.Write(log2)

	destBuf := make([]byte, 1024)
	frameType, frame, err := ReadFrame(buf, destBuf, nil)
	if err != nil {
		t.Fatalf("Failed to read log event batch frame: %v", err)
	}

	if frameType != FrameLogEventBatch {
		t.Errorf("Expected frame type 0x02, got 0x%02x", frameType)
	}

	batch, ok := frame.(*LogEventBatch)
	if !ok {
		t.Fatalf("Expected *LogEventBatch type, got %T", frame)
	}

	if batch.StreamID != streamID {
		t.Errorf("Expected StreamID %d, got %d", streamID, batch.StreamID)
	}
	if len(batch.Events) != 2 {
		t.Fatalf("Expected 2 events, got %d", len(batch.Events))
	}

	// Verify Event 1
	e1 := batch.Events[0]
	if e1.TimestampNS != 1000000001 || e1.IsStderr || !bytes.Equal(e1.Payload, log1) {
		t.Errorf("Event 1 mismatch: %+v", e1)
	}

	// Verify Event 2
	e2 := batch.Events[1]
	if e2.TimestampNS != 1000000002 || !e2.IsStderr || !bytes.Equal(e2.Payload, log2) {
		t.Errorf("Event 2 mismatch: %+v", e2)
	}

	// Verify zero-copy slicing: payloads must point directly into destBuf, not cloned
	// ReadFrame reads payloadBuf = destBuf[:payloadLen]. So event payloads must reside within destBuf.
	p1 := uintptr(unsafe.Pointer(&e1.Payload[0]))
	pStart := uintptr(unsafe.Pointer(&destBuf[0]))
	pEnd := uintptr(unsafe.Pointer(&destBuf[len(destBuf)-1]))
	if p1 < pStart || p1 >= pEnd {
		t.Error("Zero-copy violation: Event 1 payload is not sliced from destBuf")
	}
}

func TestReadFrameShortRead(t *testing.T) {
	buf := bytes.NewReader([]byte{0x00, 0x00})
	destBuf := make([]byte, 1024)
	_, _, err := ReadFrame(buf, destBuf, nil)
	if err != io.ErrUnexpectedEOF {
		t.Errorf("Expected unexpected EOF, got %v", err)
	}
}
