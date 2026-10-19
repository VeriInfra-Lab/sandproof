package main

import (
	"errors"
	"os"
	"syscall"
	"testing"
)

func TestNativePipeErrorUsesOriginalWriterName(t *testing.T) {
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	defer w.Close()
	if err := r.Close(); err != nil {
		t.Fatal(err)
	}
	if w.Name() != "|1" {
		t.Fatalf("unexpected pipe name %q", w.Name())
	}
	_, err = w.Write([]byte("not delivered"))
	var path *os.PathError
	if !errors.As(err, &path) || !errors.Is(err, syscall.EPIPE) || path.Op != "write" || path.Path != "|1" {
		t.Fatalf("unexpected original pipe failure: %v", err)
	}
}
