// Controlled observations of original generated command messages through the
// original Connect transport/default protobuf JSON codec. No process executes.
package main

import (
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"strings"

	"connectrpc.com/connect"
	rpc "sandproof.local/connect-reference/process"
)

// Byte-identical runtime f455fd58fa3f289e78ce1de690c35968d8d29322,
// packages/envd/internal/services/spec/process/process.pb.go,
// Git blob bdfc62a0aa95980ee9c15ec03a106ceaf9ff2449 (Apache-2.0, E2B-LICENSE).
//
//go:embed process/process.pb.go
var generatedProcessSource []byte

type commandResponseInput struct {
	Method    string
	Processes []struct {
		Config *struct {
			Cmd  string
			Args []string
			Envs map[string]string
			Cwd  *string
		}
		PID uint32
		Tag *string
	}
	Event struct {
		Kind     string
		PID      uint32
		Data     string // hex, preserving all bytes
		ExitCode int32
		Exited   bool
		Status   string
		Error    *string
	}
}

func commandResponseEndpoint(data []byte) (string, []byte, bool) {
	var input commandResponseInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	stream := input.Method == "Start" || input.Method == "Connect"
	body := []byte("{}")
	if stream {
		body = append([]byte{0, 0, 0, 0, 2}, body...)
	}
	return "/fixture.Process/" + input.Method, body, stream
}

func commandResponseRecord(headers http.Header) commandResponseInput {
	data, err := hex.DecodeString(headers.Get("X-Fixture-Command"))
	if err != nil {
		panic(err)
	}
	var input commandResponseInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	return input
}

func commandResponseEvent(headers http.Header) *rpc.ProcessEvent {
	v := commandResponseRecord(headers).Event
	switch v.Kind {
	case "Start":
		return &rpc.ProcessEvent{Event: &rpc.ProcessEvent_Start{Start: &rpc.ProcessEvent_StartEvent{Pid: v.PID}}}
	case "End":
		return &rpc.ProcessEvent{Event: &rpc.ProcessEvent_End{End: &rpc.ProcessEvent_EndEvent{ExitCode: v.ExitCode, Exited: v.Exited, Status: v.Status, Error: v.Error}}}
	case "Keepalive":
		return &rpc.ProcessEvent{Event: &rpc.ProcessEvent_Keepalive{Keepalive: &rpc.ProcessEvent_KeepAlive{}}}
	case "Stdout", "Stderr", "Pty":
		bytes, err := hex.DecodeString(v.Data)
		if err != nil {
			panic(err)
		}
		data := &rpc.ProcessEvent_DataEvent{}
		switch v.Kind {
		case "Stdout":
			data.Output = &rpc.ProcessEvent_DataEvent_Stdout{Stdout: bytes}
		case "Stderr":
			data.Output = &rpc.ProcessEvent_DataEvent_Stderr{Stderr: bytes}
		case "Pty":
			data.Output = &rpc.ProcessEvent_DataEvent_Pty{Pty: bytes}
		}
		return &rpc.ProcessEvent{Event: &rpc.ProcessEvent_Data{Data: data}}
	default:
		panic("unknown controlled event")
	}
}

func registerCommandResponses(mux *http.ServeMux) {
	digest := sha256.Sum256(generatedProcessSource)
	if hex.EncodeToString(digest[:]) != "e35da822dfb2870b1a6441c536b215f7a2503b37674e12a197f45f9caa083cf5" {
		panic("fixed original process protobuf source required")
	}
	const list = "/fixture.Process/List"
	mux.Handle(list, connect.NewUnaryHandler(list, func(_ context.Context, req *connect.Request[rpc.ListRequest]) (*connect.Response[rpc.ListResponse], error) {
		input := commandResponseRecord(req.Header())
		result := &rpc.ListResponse{}
		for _, v := range input.Processes {
			entry := &rpc.ProcessInfo{Pid: v.PID, Tag: v.Tag}
			if c := v.Config; c != nil {
				entry.Config = &rpc.ProcessConfig{Cmd: c.Cmd, Args: c.Args, Envs: c.Envs, Cwd: c.Cwd}
			}
			result.Processes = append(result.Processes, entry)
		}
		return connect.NewResponse(result), nil
	}))
	const start = "/fixture.Process/Start"
	mux.Handle(start, connect.NewServerStreamHandler(start, func(_ context.Context, req *connect.Request[rpc.StartRequest], stream *connect.ServerStream[rpc.StartResponse]) error {
		return stream.Send(&rpc.StartResponse{Event: commandResponseEvent(req.Header())})
	}))
	const conn = "/fixture.Process/Connect"
	mux.Handle(conn, connect.NewServerStreamHandler(conn, func(_ context.Context, req *connect.Request[rpc.ConnectRequest], stream *connect.ServerStream[rpc.ConnectResponse]) error {
		return stream.Send(&rpc.ConnectResponse{Event: commandResponseEvent(req.Header())})
	}))
	const input = "/fixture.Process/SendInput"
	mux.Handle(input, connect.NewUnaryHandler(input, func(_ context.Context, _ *connect.Request[rpc.SendInputRequest]) (*connect.Response[rpc.SendInputResponse], error) {
		return connect.NewResponse(&rpc.SendInputResponse{}), nil
	}))
	const close = "/fixture.Process/CloseStdin"
	mux.Handle(close, connect.NewUnaryHandler(close, func(_ context.Context, _ *connect.Request[rpc.CloseStdinRequest]) (*connect.Response[rpc.CloseStdinResponse], error) {
		return connect.NewResponse(&rpc.CloseStdinResponse{}), nil
	}))
	const signal = "/fixture.Process/SendSignal"
	mux.Handle(signal, connect.NewUnaryHandler(signal, func(_ context.Context, _ *connect.Request[rpc.SendSignalRequest]) (*connect.Response[rpc.SendSignalResponse], error) {
		return connect.NewResponse(&rpc.SendSignalResponse{}), nil
	}))
	const update = "/fixture.Process/Update"
	mux.Handle(update, connect.NewUnaryHandler(update, func(_ context.Context, _ *connect.Request[rpc.UpdateRequest]) (*connect.Response[rpc.UpdateResponse], error) {
		return connect.NewResponse(&rpc.UpdateResponse{}), nil
	}))
}

// Preserve flags, length validity, message presence, scalar and oneof values;
// only insignificant protobuf JSON spacing/map key order are normalized.
func commandResponseObservation(res *http.Response, body []byte) string {
	type frame struct {
		Flags   byte
		Payload json.RawMessage
	}
	result := struct {
		Status      int
		ContentType string
		Frames      []frame
	}{Status: res.StatusCode, ContentType: res.Header.Get("Content-Type")}
	if strings.HasPrefix(result.ContentType, "application/connect+") {
		for len(body) != 0 {
			if len(body) < 5 {
				panic("incomplete original Connect frame")
			}
			size := int(binary.BigEndian.Uint32(body[1:5]))
			if size > len(body)-5 {
				panic("truncated original Connect frame")
			}
			result.Frames = append(result.Frames, frame{Flags: body[0], Payload: json.RawMessage(body[5 : 5+size])})
			body = body[5+size:]
		}
	} else {
		result.Frames = []frame{{Payload: json.RawMessage(body)}}
	}
	encoded, err := json.Marshal(result)
	if err != nil {
		panic(err)
	}
	return string(encoded)
}
