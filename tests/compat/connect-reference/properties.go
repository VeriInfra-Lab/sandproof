// Test-only attributes passed to original generated messages and the unmodified
// Connect default protobuf JSON codec. No account lookup or filesystem calls.
// filesystem/filesystem.pb.go is byte-identical to runtime f455fd58fa3f289e78ce1de690c35968d8d29322,
// packages/envd/internal/services/spec/filesystem/filesystem.pb.go,
// Git blob 7724e46497b97b24e4238068360f7244b93ee2cb (Apache-2.0, E2B-LICENSE).
package main

import (
	"context"
	"crypto/sha256"
	_ "embed"
	"encoding/hex"
	"encoding/json"
	"net/http"

	"connectrpc.com/connect"
	"google.golang.org/protobuf/types/known/timestamppb"
	rpc "sandproof.local/connect-reference/filesystem"
)

//go:embed filesystem/filesystem.pb.go
var generatedFilesystemSource []byte

type propertyEntry struct {
	Name, Path, Permissions, Owner, Group string
	Type                                  int32
	Size                                  int64
	Mode                                  uint32
	Time                                  *struct {
		Seconds int64
		Nanos   int32
	}
	Link     *string
	Metadata map[string]string
}
type propertyInput struct {
	Shape   string
	Entries []propertyEntry
}

func propertyEndpoint(data []byte) string {
	var input propertyInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	switch input.Shape {
	case "Entry", "Entries", "Empty":
		return "/fixture.Properties/" + input.Shape
	default:
		panic("unknown property response shape")
	}
}
func propertyRecords(headers http.Header) []*rpc.EntryInfo {
	data, err := hex.DecodeString(headers.Get("X-Fixture-Properties"))
	if err != nil {
		panic(err)
	}
	var input propertyInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	var entries []*rpc.EntryInfo
	for _, v := range input.Entries {
		entry := &rpc.EntryInfo{Name: v.Name, Type: rpc.FileType(v.Type), Path: v.Path, Size: v.Size,
			Mode: v.Mode, Permissions: v.Permissions, Owner: v.Owner, Group: v.Group,
			SymlinkTarget: v.Link, Metadata: v.Metadata}
		if v.Time != nil {
			entry.ModifiedTime = &timestamppb.Timestamp{Seconds: v.Time.Seconds, Nanos: v.Time.Nanos}
		}
		entries = append(entries, entry)
	}
	return entries
}
func registerProperties(mux *http.ServeMux) {
	digest := sha256.Sum256(generatedFilesystemSource)
	if hex.EncodeToString(digest[:]) != "94e27e6aa36a685ea38c1ab636f2823ade538da494b476cd241f445866887b7f" {
		panic("fixed original filesystem protobuf source required")
	}
	const one = "/fixture.Properties/Entry"
	mux.Handle(one, connect.NewUnaryHandler(one, func(_ context.Context, req *connect.Request[rpc.StatRequest]) (*connect.Response[rpc.StatResponse], error) {
		entries := propertyRecords(req.Header())
		if len(entries) != 1 {
			panic("one observed entry required")
		}
		return connect.NewResponse(&rpc.StatResponse{Entry: entries[0]}), nil
	}))
	const many = "/fixture.Properties/Entries"
	mux.Handle(many, connect.NewUnaryHandler(many, func(_ context.Context, req *connect.Request[rpc.ListDirRequest]) (*connect.Response[rpc.ListDirResponse], error) {
		return connect.NewResponse(&rpc.ListDirResponse{Entries: propertyRecords(req.Header())}), nil
	}))
	const empty = "/fixture.Properties/Empty"
	mux.Handle(empty, connect.NewUnaryHandler(empty, func(_ context.Context, _ *connect.Request[rpc.RemoveRequest]) (*connect.Response[rpc.RemoveResponse], error) {
		return connect.NewResponse(&rpc.RemoveResponse{}), nil
	}))
}
