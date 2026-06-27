package protocol

import (
	"encoding/binary"
	"errors"
	"io"
)

type FrameType byte

const (
	FrameStreamRegister FrameType = 0x01
	FrameLogEventBatch  FrameType = 0x02
)

type StreamRegister struct {
	StreamID      uint32
	Namespace     string
	PodName       string
	PodUID        string
	ContainerName string
}

type LogEvent struct {
	TimestampNS int64
	IsStderr    bool
	Payload     []byte // Slices directly from the read buffer (zero-copy)
}

type LogEventBatch struct {
	StreamID uint32
	Events   []LogEvent
}

// Reset clears the events slice while keeping the underlying capacity for pool recycling.
func (b *LogEventBatch) Reset() {
	b.StreamID = 0
	b.Events = b.Events[:0]
}

// ReadFrame reads a single custom framed binary message from r.
// reuseBatch is decoded in-place when the frame type is LogEventBatch; pass nil to allocate.
// Event payloads borrow memory from destBuf for zero-copy decoding.
func ReadFrame(r io.Reader, destBuf []byte, reuseBatch *LogEventBatch) (FrameType, interface{}, error) {
	var header [5]byte
	if _, err := io.ReadFull(r, header[:]); err != nil {
		return 0, nil, err
	}

	payloadLen := binary.BigEndian.Uint32(header[0:4])
	frameType := FrameType(header[4])

	if int(payloadLen) > len(destBuf) {
		return 0, nil, errors.New("frame payload length exceeds pre-allocated destination buffer size")
	}

	// Read exact payload to guarantee atomic message boundaries
	payloadBuf := destBuf[:payloadLen]
	if _, err := io.ReadFull(r, payloadBuf); err != nil {
		return 0, nil, err
	}

	switch frameType {
	case FrameStreamRegister:
		return parseStreamRegister(payloadBuf)
	case FrameLogEventBatch:
		batch := reuseBatch
		if batch == nil {
			batch = &LogEventBatch{Events: make([]LogEvent, 0, 16)}
		}
		if err := DecodeLogEventBatch(payloadBuf, batch); err != nil {
			return 0, nil, err
		}
		return FrameLogEventBatch, batch, nil
	default:
		return 0, nil, errors.New("unknown wire frame type discriminator")
	}
}

func parseStreamRegister(data []byte) (FrameType, interface{}, error) {
	if len(data) < 12 {
		return FrameStreamRegister, nil, io.ErrUnexpectedEOF
	}

	streamID := binary.BigEndian.Uint32(data[0:4])
	idx := 4

	readString := func() (string, error) {
		if idx+2 > len(data) {
			return "", io.ErrUnexpectedEOF
		}
		strLen := int(binary.BigEndian.Uint16(data[idx : idx+2]))
		idx += 2
		if idx+strLen > len(data) {
			return "", io.ErrUnexpectedEOF
		}
		val := string(data[idx : idx+strLen])
		idx += strLen
		return val, nil
	}

	ns, err := readString()
	if err != nil {
		return FrameStreamRegister, nil, err
	}
	podName, err := readString()
	if err != nil {
		return FrameStreamRegister, nil, err
	}
	podUID, err := readString()
	if err != nil {
		return FrameStreamRegister, nil, err
	}
	containerName, err := readString()
	if err != nil {
		return FrameStreamRegister, nil, err
	}

	return FrameStreamRegister, &StreamRegister{
		StreamID:      streamID,
		Namespace:     ns,
		PodName:       podName,
		PodUID:        podUID,
		ContainerName: containerName,
	}, nil
}


// DecodeLogEventBatch parses a batch payload into dest, reusing dest.Events capacity.
// Event payloads slice directly from data (zero-copy).
func DecodeLogEventBatch(data []byte, dest *LogEventBatch) error {
	if len(data) < 8 {
		return io.ErrUnexpectedEOF
	}

	dest.StreamID = binary.BigEndian.Uint32(data[0:4])
	eventCount := binary.BigEndian.Uint32(data[4:8])
	dest.Events = dest.Events[:0]
	if cap(dest.Events) < int(eventCount) {
		dest.Events = make([]LogEvent, 0, eventCount)
	}

	idx := 8
	for i := uint32(0); i < eventCount; i++ {
		if idx+13 > len(data) {
			return io.ErrUnexpectedEOF
		}
		timestampNS := int64(binary.BigEndian.Uint64(data[idx : idx+8]))
		isStderr := data[idx+8] != 0
		payloadLen := binary.BigEndian.Uint32(data[idx+9 : idx+13])
		idx += 13

		if idx+int(payloadLen) > len(data) {
			return io.ErrUnexpectedEOF
		}
		payload := data[idx : idx+int(payloadLen)]
		idx += int(payloadLen)

		dest.Events = append(dest.Events, LogEvent{
			TimestampNS: timestampNS,
			IsStderr:    isStderr,
			Payload:     payload,
		})
	}

	return nil
}
