// Original server-stream request parsing with generated Start/Connect messages.
// The controlled handler echoes the decoded request, never executes a process.
package main

import (
	"context"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"net/http"

	"connectrpc.com/connect"
	rpc "sandproof.local/connect-reference/process"
)

func framedCommandEcho[T any](mux *http.ServeMux, method string) {
	path := "/fixture.CommandFrames/" + method
	mux.Handle(path, connect.NewServerStreamHandler(path, func(_ context.Context, req *connect.Request[T], stream *connect.ServerStream[T]) error {
		return stream.Send(req.Msg)
	}))
}
func registerFramedCommandRequests(mux *http.ServeMux) {
	framedCommandEcho[rpc.StartRequest](mux, "Start")
	framedCommandEcho[rpc.ConnectRequest](mux, "Connect")
}
func framedCommandRequestEndpoint(data []byte) (string, []byte) {
	var input struct{ Method, Body string }
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	if input.Method != "Start" && input.Method != "Connect" {
		panic("not a stream request")
	}
	body, err := hex.DecodeString(input.Body)
	if err != nil {
		panic(err)
	}
	return "/fixture.CommandFrames/" + input.Method, body
}
func framedCommandRequestObservation(res *http.Response, body []byte) string {
	if res.StatusCode != http.StatusOK {
		panic("expected Connect stream status")
	}
	var message []byte
	for len(body) > 0 {
		if len(body) < 5 {
			panic("truncated original reply")
		}
		n := int(binary.BigEndian.Uint32(body[1:5]))
		if n > len(body)-5 {
			panic("truncated original frame")
		}
		flag, payload := body[0], body[5:5+n]
		body = body[5+n:]
		if flag&1 != 0 {
			payload = commandHeaderExpand(payload)
			flag &= ^byte(1)
		}
		switch flag {
		case 0:
			if message != nil {
				panic("multiple echo replies")
			}
			message = payload
		case 2:
			if len(body) != 0 {
				panic("data after EOS")
			}
			var end struct {
				Error *struct{ Code, Message string }
			}
			if err := json.Unmarshal(payload, &end); err != nil {
				panic(err)
			}
			if end.Error != nil {
				return "ERR\t" + end.Error.Code
			}
			if message == nil {
				panic("missing echoed request")
			}
			return "OK\t" + hex.EncodeToString(message)
		default:
			panic("unexpected original flags")
		}
	}
	panic("missing EOS")
}
