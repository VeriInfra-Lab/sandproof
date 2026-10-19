// Fixed runtime f455fd58fa3f289e78ce1de690c35968d8d29322 handler.WriteStdin/
// WriteTty and process.handleInput. Controlled errno with original formatting.
// Go 1.26.8 exec.StdinPipe returns os.Pipe's writer named |1; the locked
// creack/pty v1.1.24 pty_linux.go opens the master as /dev/ptmx.
package main

import (
	"fmt"
	"os"

	"connectrpc.com/connect"
)

func nativeInputError(input commandOperationInput) error {
	var failure error
	if input.Pty {
		err := &os.PathError{Op: "write", Path: "/dev/ptmx", Err: nativeErrno(input.Native)}
		failure = fmt.Errorf("error writing to tty of process '%d': %w", uint32(input.Number), err)
		failure = fmt.Errorf("error writing to tty: %w", failure)
	} else {
		err := &os.PathError{Op: "write", Path: "|1", Err: nativeErrno(input.Native)}
		failure = fmt.Errorf("error writing to stdin of process '%d': %w", uint32(input.Number), err)
		failure = fmt.Errorf("error writing to stdin: %w", failure)
	}
	return connect.NewError(connect.CodeInternal, failure)
}
