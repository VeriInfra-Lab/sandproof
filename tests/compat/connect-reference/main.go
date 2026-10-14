// Original Connect HTTP transport pinned by envd f455fd58fa3f289e78ce1de690c35968d8d29322.
// The passthrough JSON codec isolates transport from protobuf encoding and
// actual file operations. This is not a full envd/protobuf compatibility test.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"runtime"
	"runtime/debug"
	"strconv"
	"strings"

	"connectrpc.com/connect"
)

type payload []byte
type codec struct{}

func (codec) Name() string                  { return "json" }
func (codec) Marshal(v any) ([]byte, error) { return []byte(*v.(*payload)), nil }
func (codec) Unmarshal(data []byte, v any) error {
	*v.(*payload) = append(payload(nil), data...)
	return nil
}

func main() {
	if runtime.Version() != "go1.26.8" || strconv.IntSize != 64 {
		panic("Go 1.26.8 / 64-bit int required")
	}
	info, ok := debug.ReadBuildInfo()
	if !ok {
		panic("build information required")
	}
	for path, version := range map[string]string{
		"connectrpc.com/connect": "v1.18.1", "google.golang.org/protobuf": "v1.36.12",
	} {
		found := false
		for _, dep := range info.Deps {
			if dep.Path == path && dep.Version == version && dep.Replace == nil {
				found = true
			}
		}
		if !found {
			panic("fixed original dependency required: " + path)
		}
	}
	const path = "/fixture.Filesystem/Reply"
	handler := connect.NewUnaryHandler(path, func(_ context.Context, req *connect.Request[payload]) (*connect.Response[payload], error) {
		return connect.NewResponse(req.Msg), nil
	}, connect.WithCodec(codec{}))
	server := httptest.NewServer(handler)
	defer server.Close()
	client := server.Client()
	defer client.CloseIdleConnections()
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 65536), 2<<20)
	out := bufio.NewWriter(os.Stdout)
	for scanner.Scan() {
		data, err := hex.DecodeString(scanner.Text())
		if err != nil {
			panic(err)
		}
		req, err := http.NewRequest(http.MethodPost, server.URL+path, bytes.NewReader(data))
		if err != nil {
			panic(err)
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Connect-Protocol-Version", "1")
		req.Header.Set("Accept-Encoding", "identity")
		res, err := client.Do(req)
		if err != nil {
			panic(err)
		}
		body, err := io.ReadAll(res.Body)
		if err != nil {
			panic(err)
		}
		if err := res.Body.Close(); err != nil {
			panic(err)
		}
		values := []string{strconv.Itoa(res.StatusCode)}
		for _, name := range []string{"Content-Type", "Content-Length", "Accept-Encoding", "Content-Encoding", "X-Content-Type-Options", "Vary"} {
			values = append(values, hex.EncodeToString([]byte(res.Header.Get(name))))
		}
		values = append(values, hex.EncodeToString(body))
		fmt.Fprintln(out, strings.Join(values, "\t"))
		if err := out.Flush(); err != nil {
			panic(err)
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
