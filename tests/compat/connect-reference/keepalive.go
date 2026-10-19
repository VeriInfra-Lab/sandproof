// Fixed envd permissions/keepalive.go, blob
// 3d7a0461afcbeed804dd479e4e327932e72fd65a. Apache-2.0 license at
// ../../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
// Preserve Header.Get, Atoi, Duration multiplication and actual NewTicker.
// Observation-only additions: return the duration, catch its known panic and
// stop a created ticker immediately. No reset timing or envd process is tested.
package main

import (
	"encoding/hex"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"
)

func originalKeepaliveSelection(header http.Header) (interval time.Duration, panicked bool) {
	defer func() {
		if failure := recover(); failure != nil {
			if fmt.Sprint(failure) != "non-positive interval for NewTicker" {
				panic(failure)
			}
			panicked = true
		}
	}()
	keepAliveIntervalHeader := header.Get("Keepalive-Ping-Interval")
	keepAliveIntervalInt, err := strconv.Atoi(keepAliveIntervalHeader)
	if err != nil {
		interval = 90 * time.Second
	} else {
		interval = time.Duration(keepAliveIntervalInt) * time.Second
	}
	ticker := time.NewTicker(interval)
	ticker.Stop()
	return interval, false
}

func observeKeepalive(input string) string {
	header := make(http.Header)
	if input != "-" { // Distinguish absent from one present empty field.
		for _, field := range strings.Split(input, "\t") {
			raw, err := hex.DecodeString(field)
			if err != nil {
				panic(err)
			}
			header.Add("Keepalive-Ping-Interval", string(raw))
		}
	}
	interval, panicked := originalKeepaliveSelection(header)
	if panicked {
		return fmt.Sprintf("PANIC\t%d", interval)
	}
	return fmt.Sprintf("OK\t%d", interval)
}
