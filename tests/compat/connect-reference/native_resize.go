// Fixed runtime f455fd58fa3f289e78ce1de690c35968d8d29322 process.Update.
// creack/pty v1.1.24 Setsize -> ioctlInner returns the syscall errno directly,
// without os.PathError or another handler wrapper. No ioctl is executed here.
package main

import (
	"fmt"

	"connectrpc.com/connect"
)

func nativeResizeError(input commandOperationInput) error {
	return connect.NewError(connect.CodeInternal, fmt.Errorf("error resizing tty: %w", nativeErrno(input.Native)))
}
