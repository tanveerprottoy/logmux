package ratelimit

import (
	"sync/atomic"
	"time"
)

// TokenBucket is a zero-allocation, lock-free rate limiter.
// It packs a 42-bit millisecond timestamp and a 22-bit token integer into a single uint64.
// - 42-bit millisecond timestamp: supports up to ~139 years from epoch.
// - 22-bit token counter: supports up to 4,194,303 tokens.
type TokenBucket struct {
	rate  uint32 // Tokens added per second
	burst uint32 // Maximum capacity
	state atomic.Uint64
}

const (
	tokenMask uint64 = 0x3FFFFF
	timeMask  uint64 = 0x3FFFFFFFFFF
)

// NewTokenBucket creates a rate limiter with the specified rate (tokens/sec) and burst capacity.
func NewTokenBucket(rate uint32, burst uint32) *TokenBucket {
	if burst > uint32(tokenMask) {
		burst = uint32(tokenMask)
	}
	tb := &TokenBucket{
		rate:  rate,
		burst: burst,
	}

	nowMilli := uint64(time.Now().UnixMilli()) & timeMask
	initialState := (nowMilli << 22) | uint64(burst)
	tb.state.Store(initialState)

	return tb
}

// Allow consumes n tokens from the bucket if available.
func (tb *TokenBucket) Allow(n uint32) bool {
	if n > tb.burst {
		return false
	}

	for {
		oldState := tb.state.Load()
		lastMilli := oldState >> 22
		tokens := oldState & tokenMask

		nowMilli := uint64(time.Now().UnixMilli()) & timeMask
		
		// Handle clock drift or time elapsed calculation
		var elapsedMs int64
		if nowMilli >= lastMilli {
			elapsedMs = int64(nowMilli - lastMilli)
		} else {
			// Clock wrapped or set backward slightly, treat elapsed as 0
			elapsedMs = 0
		}

		// Calculate replenished tokens: rate * (elapsedMs / 1000)
		replenished := (float64(tb.rate) * float64(elapsedMs)) / 1000.0
		newTokensVal := float64(tokens) + replenished
		if newTokensVal > float64(tb.burst) {
			newTokensVal = float64(tb.burst)
		}

		newTokens := uint32(newTokensVal)
		if newTokens < n {
			return false
		}

		// Consume n tokens
		consumedTokens := newTokens - n
		newState := (nowMilli << 22) | uint64(consumedTokens)

		if tb.state.CompareAndSwap(oldState, newState) {
			return true
		}
	}
}
