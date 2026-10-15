package main

import (
	"testing"
	"testing/synctest"
	"time"
)

// Observe the pinned toolchain's original ticker with virtual time. These
// controlled sends are not the original envd handler or an HTTP comparison.
func TestDelayedTickerAndDataReset(t *testing.T) {
	synctest.Test(t, func(t *testing.T) {
		start := time.Now()
		timer := time.NewTicker(10 * time.Second)
		defer timer.Stop()
		read := func(wantNow, wantTick time.Duration) {
			tick := <-timer.C
			t.Logf("receive at %s; tick %s", time.Since(start), tick.Sub(start))
			if time.Since(start) != wantNow || tick.Sub(start) != wantTick {
				t.Fatal("unexpected tick")
			}
		}
		read(10*time.Second, 10*time.Second)
		time.Sleep(24 * time.Second) // keepalive Send delayed, without reset
		read(34*time.Second, 20*time.Second)
		read(40*time.Second, 40*time.Second)
		time.Sleep(3 * time.Second)
		timer.Reset(10 * time.Second) // data Send completed at 43
		read(53*time.Second, 53*time.Second)
		time.Sleep(24 * time.Second)
		timer.Reset(10 * time.Second) // data cancels the missed tick at 63
		read(87*time.Second, 87*time.Second)
	})
}
