// Original command message decoding through Connect's default JSON codec.
// All seven fixtures are unary echoes of the decoded request, including the
// Start/Connect request types. This isolates JSON decoding, not streaming
// framing, method dispatch, authentication, or process execution.
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

func commandRequestEcho[T any](mux *http.ServeMux, method string) {
	path := "/fixture.CommandRequests/" + method
	mux.Handle(path, connect.NewUnaryHandler(path, func(_ context.Context, req *connect.Request[T]) (*connect.Response[T], error) {
		return connect.NewResponse(req.Msg), nil
	}))
}
func registerCommandRequests(mux *http.ServeMux) {
	commandRequestEcho[rpc.StartRequest](mux, "Start")
	commandRequestEcho[rpc.ConnectRequest](mux, "Connect")
	commandRequestEcho[rpc.ListRequest](mux, "List")
	commandRequestEcho[rpc.SendInputRequest](mux, "SendInput")
	commandRequestEcho[rpc.CloseStdinRequest](mux, "CloseStdin")
	commandRequestEcho[rpc.SendSignalRequest](mux, "SendSignal")
	commandRequestEcho[rpc.UpdateRequest](mux, "Update")
}
func commandRequestEndpoint(data []byte) (string, []byte) {
	var input struct{ Method, Body string }
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	switch input.Method {
	case "Start", "Connect", "List", "SendInput", "CloseStdin", "SendSignal", "Update":
	default:
		panic("unknown command request")
	}
	body, err := hex.DecodeString(input.Body)
	if err != nil {
		panic(err)
	}
	return "/fixture.CommandRequests/" + input.Method, body
}
func commandRequestObservation(status int, body []byte) string {
	if status == http.StatusBadRequest {
		var failure struct{ Code string }
		if err := json.Unmarshal(body, &failure); err != nil || failure.Code != "invalid_argument" {
			panic("unexpected command rejection")
		}
		return "ERR\tinvalid_argument"
	}
	if status != http.StatusOK {
		panic(fmt.Sprintf("unexpected command request status %d: %s", status, body))
	}
	return "OK\t" + hex.EncodeToString(body)
}
