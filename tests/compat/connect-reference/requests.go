// Request observations through the original generated messages and unmodified
// Connect codec. Echo handlers deliberately perform no account or file I/O.
package main

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"

	"connectrpc.com/connect"
	"google.golang.org/protobuf/reflect/protoreflect"
	rpc "sandproof.local/connect-reference/filesystem"
)

func requestEcho[T any](mux *http.ServeMux, method string) {
	path := "/fixture.Requests/" + method
	mux.Handle(path, connect.NewUnaryHandler(path, func(_ context.Context, req *connect.Request[T]) (*connect.Response[T], error) {
		return connect.NewResponse(req.Msg), nil
	}, connect.WithSchema(rpc.File_filesystem_filesystem_proto.Services().ByName("Filesystem").Methods().ByName(protoreflect.Name(method)))))
}

func normalizeRequestFailure(target string, status int, body []byte) []byte {
	var fields map[string]json.RawMessage
	var failure struct {
		Code    string `json:"code"`
		Message string `json:"message"`
	}
	if status != http.StatusBadRequest || json.Unmarshal(body, &fields) != nil || len(fields) != 2 || json.Unmarshal(body, &failure) != nil || failure.Code != "invalid_argument" {
		panic("expected original JSON failure")
	}
	method := strings.TrimPrefix(target, "/fixture.Requests/")
	prefix := "unmarshal message: unmarshal into *filesystem." + method + "Request: proto:"
	if strings.HasPrefix(failure.Message, prefix+"\u00a0") {
		old := []byte(`"message":"` + prefix + "\u00a0")
		if bytes.Count(body, old) != 1 {
			panic("ambiguous protobuf prefix")
		}
		return bytes.Replace(body, old, []byte(`"message":"`+prefix+" "), 1)
	}
	return body
}
func registerRequests(mux *http.ServeMux) {
	requestEcho[rpc.StatRequest](mux, "Stat")
	requestEcho[rpc.MakeDirRequest](mux, "MakeDir")
	requestEcho[rpc.ListDirRequest](mux, "ListDir")
	requestEcho[rpc.MoveRequest](mux, "Move")
	requestEcho[rpc.RemoveRequest](mux, "Remove")
}
func requestEndpoint(data []byte) (string, []byte) {
	var input struct{ Method, Body string }
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	switch input.Method {
	case "Stat", "MakeDir", "ListDir", "Move", "Remove":
	default:
		panic("unknown request reference method")
	}
	body, err := hex.DecodeString(input.Body)
	if err != nil {
		panic(err)
	}
	return "/fixture.Requests/" + input.Method, body
}
func requestObservation(status int, body []byte) string {
	if status == http.StatusBadRequest {
		var failure struct{ Code string }
		if err := json.Unmarshal(body, &failure); err != nil || failure.Code != "invalid_argument" {
			panic("unexpected request rejection")
		}
		return "ERR\tinvalid_argument"
	}
	if status != http.StatusOK {
		panic(fmt.Sprintf("unexpected request status %d: %s", status, body))
	}
	var fields struct {
		Path, Source, Destination string
		Depth                     uint32
	}
	if err := json.Unmarshal(body, &fields); err != nil {
		panic(err)
	}
	return fmt.Sprintf("OK\t%s\t%d\t%s\t%s", hex.EncodeToString([]byte(fields.Path)), fields.Depth,
		hex.EncodeToString([]byte(fields.Source)), hex.EncodeToString([]byte(fields.Destination)))
}
