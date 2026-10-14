// Test-only stdin/stdout adapter over the unmodified fixed Go filesystem
// service. Build with an overlay into packages/envd; never run envd main.go.
// No HTTP listener, cgroup manager, MMDS, freeze loop or guest bootstrap.
package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/user"

	"connectrpc.com/connect"
	"github.com/e2b-dev/infra/packages/envd/internal/execcontext"
	"github.com/e2b-dev/infra/packages/envd/internal/services/filesystem"
	rpc "github.com/e2b-dev/infra/packages/envd/internal/services/spec/filesystem"
	"github.com/go-chi/chi/v5"
	"github.com/rs/zerolog"
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/proto"
)

type request struct {
	Method      string
	Path        string
	Source      string
	Destination string
	Depth       uint32
}

func main() {
	u, err := user.Current()
	if err != nil {
		panic(err)
	}
	logger := zerolog.Nop()
	service := filesystem.Handle(chi.NewRouter(), &logger, &execcontext.Defaults{User: u.Username})
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 4096), 65536)
	for scanner.Scan() {
		var input request
		if err := json.Unmarshal(scanner.Bytes(), &input); err != nil {
			panic(err)
		}
		message, err := call(service, input)
		if err != nil {
			b, _ := json.Marshal(map[string]string{"error": connect.CodeOf(err).String()})
			fmt.Println(string(b))
			continue
		}
		b, err := (protojson.MarshalOptions{EmitDefaultValues: true}).Marshal(message)
		if err != nil {
			fmt.Println(`{"error":"internal"}`)
			continue
		}
		fmt.Println(string(b))
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}

func call(s filesystem.Service, r request) (proto.Message, error) {
	ctx := context.Background()
	switch r.Method {
	case "Stat":
		v, e := s.Stat(ctx, connect.NewRequest(&rpc.StatRequest{Path: r.Path}))
		if e != nil {
			return nil, e
		}
		return v.Msg, nil
	case "MakeDir":
		v, e := s.MakeDir(ctx, connect.NewRequest(&rpc.MakeDirRequest{Path: r.Path}))
		if e != nil {
			return nil, e
		}
		return v.Msg, nil
	case "ListDir":
		v, e := s.ListDir(ctx, connect.NewRequest(&rpc.ListDirRequest{Path: r.Path, Depth: r.Depth}))
		if e != nil {
			return nil, e
		}
		return v.Msg, nil
	case "Move":
		v, e := s.Move(ctx, connect.NewRequest(&rpc.MoveRequest{Source: r.Source, Destination: r.Destination}))
		if e != nil {
			return nil, e
		}
		return v.Msg, nil
	case "Remove":
		v, e := s.Remove(ctx, connect.NewRequest(&rpc.RemoveRequest{Path: r.Path}))
		if e != nil {
			return nil, e
		}
		return v.Msg, nil
	default:
		return nil, connect.NewError(connect.CodeUnimplemented, fmt.Errorf("unknown method"))
	}
}
