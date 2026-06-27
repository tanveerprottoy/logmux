package ratelimit

import (
	"testing"
	"time"
)

func TestTokenBucketLimiter(t *testing.T) {
	// Rate = 100/sec, Burst = 5
	tb := NewTokenBucket(100, 5)

	// Consume all 5 burst tokens
	for i := 0; i < 5; i++ {
		if !tb.Allow(1) {
			t.Errorf("Expected token %d to be allowed", i+1)
		}
	}

	// 6th should be rejected immediately
	if tb.Allow(1) {
		t.Error("Expected 6th token to be blocked (bucket exhausted)")
	}

	// Sleep 21 milliseconds. At 100 tokens/sec, this should replenish 2 tokens.
	// Calculation: 100 * (21 / 1000) = 2.1 tokens.
	time.Sleep(21 * time.Millisecond)

	if !tb.Allow(1) {
		t.Error("Expected token to be allowed after sleep (replenished)")
	}
	if !tb.Allow(1) {
		t.Error("Expected second token to be allowed after sleep")
	}
	if tb.Allow(1) {
		t.Error("Expected third token to be blocked (limit reached after partial replenishment)")
	}
}

func TestTokenBucketLimiterLargeBurst(t *testing.T) {
	// Requesting more than burst capacity should be rejected immediately
	tb := NewTokenBucket(10, 5)
	if tb.Allow(6) {
		t.Error("Should block consumption requests exceeding burst capacity")
	}
}
