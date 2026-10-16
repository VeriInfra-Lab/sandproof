// Controlled failure observations from runtime f455fd58fa3f289e78ce1de690c35968d8d29322:
// process/service.go getProcess, input.go handleInput/CloseStdin, signal.go
// SendSignal, update.go Update, and handler/handler.go terminal capabilities
// and missing-cwd expression (a controlled Stat ENOENT observation).
// Original fmt/generated enum/Connect perform rendering; this does not execute
// those service methods, query an OS process table or validate their ordering.
package main

import (
	"connectrpc.com/connect"
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"os/user"
	rpc "sandproof.local/connect-reference/process"
)

type commandOperationInput struct {
	Method, Kind string
	Cwd          string
	User         string
	DefaultCwd   *string
	Cmd, Native  string
	Args         []string
	Env          map[string]string
	Pty          bool
	Cols, Rows   uint32
	Number       int64
	Gzip, UTF8   bool
}

func operationRecord(data []byte) commandOperationInput {
	var input commandOperationInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	return input
}
func operationError(headers http.Header) error {
	data, err := hex.DecodeString(headers.Get("X-Fixture-Operation"))
	if err != nil {
		panic(err)
	}
	input := operationRecord(data)
	switch input.Kind {
	case "nul":
		return nulStartError(input)
	case "native-resize":
		return nativeResizeError(input)
	case "native-signal":
		return nativeSignalError(input)
	case "native-input":
		return nativeInputError(input)
	case "expansion":
		_, err := originalExpandAndResolve(input.Cwd, &user.User{Username: input.User, HomeDir: "/home/fixture"}, input.DefaultCwd)
		if err == nil {
			panic("expected path expansion failure")
		}
		// handler.New propagates this exact error as CodeInvalidArgument.
		return connect.NewError(connect.CodeInvalidArgument, err)
	case "native":
		return nativeStartError(input)
	case "cwd":
		return connect.NewError(connect.CodeInvalidArgument, fmt.Errorf("cwd '%s' does not exist", input.Cwd))
	case "selector":
		var selector *rpc.ProcessSelector
		return connect.NewError(connect.CodeUnimplemented, fmt.Errorf("invalid input type %T", selector))
	case "pid":
		return connect.NewError(connect.CodeNotFound, fmt.Errorf("process with pid %d not found", uint32(input.Number)))
	case "input":
		var input *rpc.ProcessInput
		return connect.NewError(connect.CodeUnimplemented, fmt.Errorf("invalid input type %T", input.GetInput()))
	case "signal":
		return connect.NewError(connect.CodeUnimplemented, fmt.Errorf("invalid signal: %s", rpc.Signal(input.Number)))
	case "stdin-on-pty":
		return connect.NewError(connect.CodeInternal, fmt.Errorf("error writing to stdin: %w", errors.New("tty assigned to process — input should be written to the pty, not the stdin")))
	case "stdin-closed":
		return connect.NewError(connect.CodeInternal, fmt.Errorf("error writing to stdin: %w", errors.New("stdin not enabled or closed")))
	case "pty-on-pipe":
		return connect.NewError(connect.CodeInternal, fmt.Errorf("error writing to tty: %w", errors.New("tty not assigned to process — input should be written to the stdin, not the tty")))
	case "close-pty":
		return connect.NewError(connect.CodeUnknown, fmt.Errorf("error closing stdin: %w", errors.New("cannot close stdin for PTY process — send Ctrl+D (0x04) instead")))
	case "resize-pipe":
		return connect.NewError(connect.CodeInternal, fmt.Errorf("error resizing tty: %w", errors.New("tty not assigned to process")))
	default:
		panic("unknown operation failure")
	}
}
func operationUnary[T any, U any](mux *http.ServeMux, method string) {
	path := "/fixture.CommandOperations/" + method
	mux.Handle(path, connect.NewUnaryHandler(path, func(_ context.Context, req *connect.Request[T]) (*connect.Response[U], error) {
		return nil, operationError(req.Header())
	}))
}
func registerCommandOperations(mux *http.ServeMux) {
	start := "/fixture.CommandOperations/Start"
	mux.Handle(start, connect.NewServerStreamHandler(start, func(_ context.Context, req *connect.Request[rpc.StartRequest], _ *connect.ServerStream[rpc.StartResponse]) error {
		return operationError(req.Header())
	}))
	path := "/fixture.CommandOperations/Connect"
	mux.Handle(path, connect.NewServerStreamHandler(path, func(_ context.Context, req *connect.Request[rpc.ConnectRequest], _ *connect.ServerStream[rpc.ConnectResponse]) error {
		return operationError(req.Header())
	}))
	operationUnary[rpc.SendInputRequest, rpc.SendInputResponse](mux, "SendInput")
	operationUnary[rpc.SendSignalRequest, rpc.SendSignalResponse](mux, "SendSignal")
	operationUnary[rpc.CloseStdinRequest, rpc.CloseStdinResponse](mux, "CloseStdin")
	operationUnary[rpc.UpdateRequest, rpc.UpdateResponse](mux, "Update")
}
func operationEndpoint(data []byte) (string, []byte) {
	input := operationRecord(data)
	body := []byte("{}")
	if input.Method == "Connect" || input.Method == "Start" {
		body = append([]byte{0, 0, 0, 0, 2}, body...)
	}
	return "/fixture.CommandOperations/" + input.Method, body
}
func operationHeaders(req *http.Request, data []byte) {
	input := operationRecord(data)
	accept := "Accept-Encoding"
	if input.Method == "Connect" || input.Method == "Start" {
		req.Header.Set("Content-Type", "application/connect+json")
		accept = "Connect-Accept-Encoding"
	}
	if input.Gzip {
		req.Header.Set(accept, "gzip")
	}
	if input.UTF8 {
		req.Header.Set("Content-Type", req.Header.Get("Content-Type")+"; charset=utf-8")
	}
	req.Header.Set("X-Fixture-Operation", hex.EncodeToString(data))
}
