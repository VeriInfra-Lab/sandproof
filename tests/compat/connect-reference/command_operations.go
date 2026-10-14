// Controlled failure observations from runtime f455fd58fa3f289e78ce1de690c35968d8d29322:
// process/service.go getProcess, input.go handleInput, signal.go SendSignal.
// Original fmt/generated enum/Connect perform rendering; this does not execute
// those service methods, query an OS process table or validate their ordering.
package main

import (
	"connectrpc.com/connect"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	rpc "sandproof.local/connect-reference/process"
)

type commandOperationInput struct {
	Method, Kind string
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
	if input.Method == "Connect" {
		body = append([]byte{0, 0, 0, 0, 2}, body...)
	}
	return "/fixture.CommandOperations/" + input.Method, body
}
func operationHeaders(req *http.Request, data []byte) {
	input := operationRecord(data)
	accept := "Accept-Encoding"
	if input.Method == "Connect" {
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
