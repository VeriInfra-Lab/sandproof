// Original Connect HTTP transport pinned by envd f455fd58fa3f289e78ce1de690c35968d8d29322.
// Transport modes use a passthrough JSON codec. P/J/K/O/Q use original generated
// messages and the default codec for property/command observations. No mode
// here executes real file operations or a complete envd compatibility test.
package main

import (
	"bufio"
	"bytes"
	"compress/gzip"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/user"
	"runtime"
	"runtime/debug"
	"strconv"
	"strings"

	"connectrpc.com/authn"
	"connectrpc.com/connect"
)

type payload []byte
type codec struct{ name string }

func (c codec) Name() string {
	if c.name != "" {
		return c.name
	}
	return "json"
}
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
		"connectrpc.com/authn": "v0.1.0", "connectrpc.com/connect": "v1.18.1", "google.golang.org/protobuf": "v1.36.12",
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
	}, connect.WithCodec(codec{}), connect.WithCodec(codec{name: "json; charset=utf-8"}))
	mux := http.NewServeMux()
	mux.Handle(path, handler)
	registerProperties(mux)
	registerRequests(mux)
	registerCommandResponses(mux)
	registerCommandRequests(mux)
	// The envd account failure observation is controlled here. Both wrappers
	// below are the original authn/Connect implementation, not a replacement.
	mux.Handle("/fixture.Filesystem/Explicit", authn.NewMiddleware(func(_ context.Context, req authn.Request) (any, error) {
		username, _, ok := req.BasicAuth()
		if !ok {
			panic("reference explicit user required")
		}
		return nil, authn.Errorf("invalid username: '%s'", username)
	}).Wrap(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { panic("rejected user reached method") })))
	mux.Handle("/fixture.Filesystem/Default", connect.NewUnaryHandler("/fixture.Filesystem/Default", func(_ context.Context, req *connect.Request[payload]) (*connect.Response[payload], error) {
		username, err := hex.DecodeString(req.Header().Get("X-Fixture-Username"))
		if err != nil {
			panic(err)
		}
		return nil, authn.Errorf("invalid default user: '%s'", username)
	}, connect.WithCodec(codec{})))
	mux.Handle("/fixture.Filesystem/Expand", connect.NewUnaryHandler("/fixture.Filesystem/Expand", func(_ context.Context, req *connect.Request[payload]) (*connect.Response[payload], error) {
		var input struct{ Name, Path, Default string }
		if err := json.Unmarshal(*req.Msg, &input); err != nil {
			panic(err)
		}
		_, err := originalExpandAndResolve(input.Path, &user.User{Username: input.Name, HomeDir: "/synthetic/home"}, &input.Default)
		if err == nil {
			panic("path reference requires an actual expansion rejection")
		}
		return nil, connect.NewError(connect.CodeInvalidArgument, err)
	}, connect.WithCodec(codec{})))
	server := httptest.NewServer(mux)
	defer server.Close()
	client := server.Client()
	client.Transport.(*http.Transport).DisableCompression = true
	defer client.CloseIdleConnections()
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 65536), 2<<20)
	out := bufio.NewWriter(os.Stdout)
	for scanner.Scan() {
		if value, ok := strings.CutPrefix(scanner.Text(), "B\t"); ok {
			raw, err := hex.DecodeString(value)
			if err != nil {
				panic(err)
			}
			req := &http.Request{Header: make(http.Header)}
			req.Header.Set("Authorization", string(raw))
			user, _, valid := req.BasicAuth()
			if valid {
				fmt.Fprintf(out, "OK\t%s\n", hex.EncodeToString([]byte(user)))
			} else {
				fmt.Fprintln(out, "NONE")
			}
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if value, ok := strings.CutPrefix(scanner.Text(), "T\t"); ok {
			raw, err := hex.DecodeString(value)
			if err != nil {
				panic(err)
			}
			// Same first-value, empty, ten-byte and strconv.ParseInt decisions
			// as v1.18.1 protocol_connect.go SetTimeout, before context creation.
			// Report parsed milliseconds; do not race a live wall-clock deadline.
			switch {
			case len(raw) == 0:
				fmt.Fprintln(out, "NONE")
			case len(raw) > 10:
				fmt.Fprintln(out, "ERR\tlength")
			default:
				millis, err := strconv.ParseInt(string(raw), 10, 64)
				if err != nil {
					fmt.Fprintln(out, "ERR\tinvalid")
				} else {
					fmt.Fprintf(out, "OK\t%d\n", millis)
				}
			}
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		mode, input, special := strings.Cut(scanner.Text(), "\t")
		if !special {
			input = scanner.Text()
		}
		data, err := hex.DecodeString(input)
		if err != nil {
			panic(err)
		}
		target, body := path, data
		commandStream := false
		var compression struct {
			Accept []string
			Sent   []string
			Data   string
			Error  bool
		}
		switch mode {
		case "Q":
			target, body = commandRequestEndpoint(data)
		case "O":
			target, body, commandStream = commandResponseEndpoint(data)
		case "J", "K":
			target, body = requestEndpoint(data)
		case "M":
			target, body = methodEndpoint(data)
		case "V", "W":
			target, body = commandHeaderEndpoint(data)
		case "P":
			target, body = propertyEndpoint(data), []byte("{}")
		case "A":
			target, body = "/fixture.Filesystem/Explicit", []byte("invalid JSON")
		case "U":
			target, body = "/fixture.Filesystem/Default", []byte("{}")
		case "X":
			target = "/fixture.Filesystem/Expand"
		case "G", "Z":
			if err := json.Unmarshal(data, &compression); err != nil {
				panic(err)
			}
			body, err = hex.DecodeString(compression.Data)
			if err != nil {
				panic(err)
			}
			if compression.Error {
				target = "/fixture.Filesystem/Expand"
			}
		case "H":
			body = []byte("invalid JSON")
		case "C":
			body = []byte("{}")
		}
		req, err := http.NewRequest(http.MethodPost, server.URL+target, bytes.NewReader(body))
		if err != nil {
			panic(err)
		}
		req.Header.Set("Content-Type", "application/json")
		req.Header.Set("Connect-Protocol-Version", "1")
		req.Header.Set("Accept-Encoding", "identity")
		if mode == "O" {
			req.Header.Set("X-Fixture-Command", hex.EncodeToString(data))
			if commandStream {
				req.Header.Set("Content-Type", "application/connect+json")
			}
		}
		if mode == "M" {
			methodHeaders(req, data)
		}
		if mode == "V" || mode == "W" {
			commandHeaders(req, data)
		}
		if mode == "G" || mode == "Z" {
			req.Header.Del("Accept-Encoding")
			for _, value := range compression.Accept {
				req.Header.Add("Accept-Encoding", value)
			}
			for _, value := range compression.Sent {
				req.Header.Add("Content-Encoding", value)
			}
		}
		if mode == "P" {
			req.Header.Set("X-Fixture-Properties", hex.EncodeToString(data))
		} else if mode == "C" {
			req.Header.Set("Content-Type", string(data))
		} else if mode == "A" {
			req.SetBasicAuth(string(data), "ignored-password")
			req.Header.Set("Connect-Timeout-Ms", "invalid")
		} else if mode == "U" {
			req.Header.Set("X-Fixture-Username", hex.EncodeToString(data))
		} else if mode == "H" {
			var fields struct {
				Version     []string `json:"version"`
				Timeout     []string `json:"timeout"`
				Compression []string `json:"compression"`
			}
			if err := json.Unmarshal(data, &fields); err != nil {
				panic(err)
			}
			for name, values := range map[string][]string{
				"Connect-Protocol-Version": fields.Version,
				"Connect-Timeout-Ms":       fields.Timeout,
				"Content-Encoding":         fields.Compression,
			} {
				req.Header.Del(name)
				for _, value := range values {
					raw, err := hex.DecodeString(value)
					if err != nil {
						panic(err)
					}
					req.Header.Add(name, string(raw))
				}
			}
		}
		res, err := client.Do(req)
		if err != nil {
			panic(err)
		}
		responseBody, err := io.ReadAll(res.Body)
		if err != nil {
			panic(err)
		}
		if err := res.Body.Close(); err != nil {
			panic(err)
		}
		if mode == "O" {
			fmt.Fprintln(out, commandResponseObservation(res, responseBody))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if mode == "V" {
			fmt.Fprintln(out, commandHeaderObservation(res, responseBody))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if mode == "W" {
			fmt.Fprintln(out, commandHeaderReply(res, responseBody))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if mode == "Q" {
			fmt.Fprintln(out, commandRequestObservation(res.StatusCode, responseBody))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if mode == "J" {
			fmt.Fprintln(out, requestObservation(res.StatusCode, responseBody))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if mode == "K" {
			if length := res.Header.Get("Content-Length"); length != "" {
				if length != strconv.Itoa(len(responseBody)) {
					panic("invalid JSON rejection response length")
				}
				res.Header.Set("Content-Length", "valid")
			}
			responseBody = normalizeRequestFailure(target, res.StatusCode, responseBody)
		}
		if mode == "P" {
			// Original protojson deliberately varies insignificant whitespace.
			// Check actual wire length first, then compare compact JSON. This does
			// not claim identical near-2048 Content-Length header thresholds.
			if length := res.Header.Get("Content-Length"); length != "" {
				if length != strconv.Itoa(len(responseBody)) {
					panic("invalid property response length")
				}
				res.Header.Set("Content-Length", "valid")
			}
			var compact bytes.Buffer
			if err := json.Compact(&compact, responseBody); err != nil {
				panic(err)
			}
			responseBody = compact.Bytes()
		}
		if mode == "G" || mode == "Z" {
			// Compare semantic gzip completion, never DEFLATE implementation bytes.
			if length := res.Header.Get("Content-Length"); length != "" {
				if length != strconv.Itoa(len(responseBody)) {
					panic("invalid response length")
				}
				res.Header.Set("Content-Length", "valid")
			}
			if res.Header.Get("Content-Encoding") == "gzip" {
				reader, err := gzip.NewReader(bytes.NewReader(responseBody))
				if err != nil {
					panic(err)
				}
				responseBody, err = io.ReadAll(reader)
				if err != nil {
					panic(err)
				}
				if err := reader.Close(); err != nil {
					panic(err)
				}
			}
		}
		if mode == "Z" {
			// Successful original Connect request decompression + response selection.
			// Passthrough codec isolates this from protobuf parsing/file methods.
			fmt.Fprintf(out, "%d\t%s\t%s\n", res.StatusCode, hex.EncodeToString([]byte(res.Header.Get("Content-Encoding"))), hex.EncodeToString(responseBody))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		if mode == "C" {
			selected := "OTHER"
			if res.StatusCode == http.StatusOK {
				switch res.Header.Get("Content-Type") {
				case "application/json":
					selected = "JSON"
				case "application/json; charset=utf-8":
					selected = "UTF8"
				}
			}
			fmt.Fprintln(out, selected)
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		values := []string{strconv.Itoa(res.StatusCode)}
		if mode == "M" {
			for _, name := range []string{"Allow", "Accept-Post"} {
				values = append(values, hex.EncodeToString([]byte(res.Header.Get(name))))
			}
		}
		for _, name := range []string{"Content-Type", "Content-Length", "Accept-Encoding", "Content-Encoding", "X-Content-Type-Options", "Vary"} {
			values = append(values, hex.EncodeToString([]byte(res.Header.Get(name))))
		}
		values = append(values, hex.EncodeToString(responseBody))
		fmt.Fprintln(out, strings.Join(values, "\t"))
		if err := out.Flush(); err != nil {
			panic(err)
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
