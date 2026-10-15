// Fixed runtime f455fd58fa3f289e78ce1de690c35968d8d29322
// process/handler/handler.go New/Start and start_error.go StartErrorCode.
// Controlled native error facts, original fmt/os.PathError/Connect rendering;
// no original process starts. Symbolic errno names avoid using Linux numbers
// as Darwin syscall values when the explicit oracle runs on macOS.
package main

import (
	"context"
	"errors"
	"fmt"
	"os"
	"strings"
	"syscall"

	"connectrpc.com/connect"
)

func startErrorCode(err error) connect.Code {
	switch {
	case errors.Is(err, syscall.EAGAIN), errors.Is(err, syscall.ENOMEM),
		errors.Is(err, syscall.EMFILE), errors.Is(err, syscall.ENFILE), errors.Is(err, syscall.ENOSPC):
		return connect.CodeResourceExhausted
	case errors.Is(err, context.DeadlineExceeded):
		return connect.CodeDeadlineExceeded
	case errors.Is(err, context.Canceled):
		return connect.CodeCanceled
	default:
		return connect.CodeInvalidArgument
	}
}

func nativeStartError(input commandOperationInput) error {
	numbers := map[string]syscall.Errno{
		"EPERM": syscall.EPERM, "ENOENT": syscall.ENOENT, "EIO": syscall.EIO,
		"E2BIG": syscall.E2BIG, "ENOEXEC": syscall.ENOEXEC, "EBADF": syscall.EBADF,
		"EAGAIN": syscall.EAGAIN, "ENOMEM": syscall.ENOMEM, "EACCES": syscall.EACCES,
		"ENOTDIR": syscall.ENOTDIR, "EINVAL": syscall.EINVAL, "ENFILE": syscall.ENFILE,
		"EMFILE": syscall.EMFILE, "ENOSPC": syscall.ENOSPC, "ENAMETOOLONG": syscall.ENAMETOOLONG,
		"ELOOP": syscall.ELOOP, "UNKNOWN": syscall.Errno(2147483647),
	}
	number, ok := numbers[input.Native]
	if !ok {
		panic("unknown fixture native errno")
	}
	native := &os.PathError{Op: "fork/exec", Path: "/bin/sh", Err: number}
	command := strings.Join(append([]string{input.Cmd}, input.Args...), " ")
	var failure error
	if input.Pty {
		failure = fmt.Errorf("error starting pty with command '%s' in dir '%s' with '%d' cols and '%d' rows: %w", command, input.Cwd, input.Cols, input.Rows, native)
	} else {
		failure = fmt.Errorf("error starting process '%s': %w", command, native)
	}
	return connect.NewError(startErrorCode(failure), failure)
}
