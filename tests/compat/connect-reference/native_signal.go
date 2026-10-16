// Fixed runtime f455fd58fa3f289e78ce1de690c35968d8d29322 process.SendSignal
// rendering. Go 1.26.8 os.Process.Signal's convertESRCH is applied first.
// The errno is controlled; this does not signal a process or query a PID.
package main

import (
	"fmt"
	"os"
	"syscall"

	"connectrpc.com/connect"
)

func nativeSignalError(input commandOperationInput) error {
	var err error = nativeErrno(input.Native)
	if err == syscall.ESRCH {
		err = os.ErrProcessDone
	}
	return connect.NewError(connect.CodeInternal, fmt.Errorf("error sending signal: %w", err))
}
