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

func nativeErrno(name string) syscall.Errno {
	numbers := map[string]syscall.Errno{
		"EPERM": syscall.EPERM, "ENOENT": syscall.ENOENT, "EIO": syscall.EIO,
		"E2BIG": syscall.E2BIG, "ENOEXEC": syscall.ENOEXEC, "EBADF": syscall.EBADF,
		"EAGAIN": syscall.EAGAIN, "ENOMEM": syscall.ENOMEM, "EACCES": syscall.EACCES,
		"ENOTDIR": syscall.ENOTDIR, "EINVAL": syscall.EINVAL, "ENFILE": syscall.ENFILE,
		"EMFILE": syscall.EMFILE, "ENOSPC": syscall.ENOSPC, "ENAMETOOLONG": syscall.ENAMETOOLONG,
		"ELOOP": syscall.ELOOP, "UNKNOWN": syscall.Errno(2147483647),
		"EPIPE": syscall.EPIPE, "EINTR": syscall.EINTR,
		"ESRCH": syscall.ESRCH, "ENOSYS": syscall.ENOSYS,
		"ENOTTY": syscall.ENOTTY,
	}
	number, ok := numbers[name]
	if !ok {
		panic("unknown fixture native errno")
	}
	return number
}

func nativeStartError(input commandOperationInput) error {
	number := nativeErrno(input.Native)
	native := &os.PathError{Op: "fork/exec", Path: "/bin/sh", Err: number}
	return formatStartError(input, native)
}

func pipeStartError(input commandOperationInput) error {
	if input.Pty || (input.Pipe != "stdin" && input.Pipe != "stdout" && input.Pipe != "stderr") {
		panic("invalid pipe fixture")
	}
	command := strings.Join(append([]string{input.Cmd}, input.Args...), " ")
	// Fixed Go Linux os.Pipe uses Pipe2 and names this syscall in its error.
	native := os.NewSyscallError("pipe2", nativeErrno(input.Native))
	failure := fmt.Errorf("error creating %s pipe for command '%s': %w", input.Pipe, command, native)
	return connect.NewError(connect.CodeInternal, failure)
}

// The pinned envd uses creack/pty v1.1.24: Linux open() returns the
// os.OpenFile /dev/ptmx error, while unlockpt and Setsize return bare ioctl
// errno. Controlled facts here do not execute those terminal operations.
func ptyPrepareError(input commandOperationInput) error {
	if !input.Pty {
		panic("invalid PTY fixture")
	}
	var native error = nativeErrno(input.Native)
	switch input.Stage {
	case "open-master":
		native = &os.PathError{Op: "open", Path: "/dev/ptmx", Err: native}
	case "unlock", "resize":
	default:
		panic("invalid PTY preparation stage")
	}
	return formatStartError(input, native)
}

func formatStartError(input commandOperationInput, native error) error {
	command := strings.Join(append([]string{input.Cmd}, input.Args...), " ")
	var failure error
	if input.Pty {
		failure = fmt.Errorf("error starting pty with command '%s' in dir '%s' with '%d' cols and '%d' rows: %w", command, input.Cwd, input.Cols, input.Rows, native)
	} else {
		failure = fmt.Errorf("error starting process '%s': %w", command, native)
	}
	return connect.NewError(startErrorCode(failure), failure)
}
