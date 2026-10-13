// Fixed envd api.gen.go GetFiles/PostFiles parameter binding order at
// f455fd58fa3f289e78ce1de690c35968d8d29322 (Apache-2.0, license retained at
// ../../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE).
// Calls the original URL.Query and locked oapi runtime v1.6.0 binder; no files,
// auth or full envd server. HTTP mode observes Go's real http.Error transport.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"runtime"
	"runtime/debug"
	"strconv"
	"strings"

	oapi "github.com/oapi-codegen/runtime"
)

func bind(raw string) (string, error) {
	query := (&url.URL{RawQuery: raw}).Query()
	var path, username, signature *string
	var expiration *int
	for _, p := range []struct {
		name, kind string
		target     any
	}{
		{"path", "string", &path}, {"username", "string", &username},
		{"signature", "string", &signature}, {"signature_expiration", "integer", &expiration},
	} {
		err := oapi.BindQueryParameterWithOptions("form", true, false, p.name, query, p.target, oapi.BindQueryParameterOptions{Type: p.kind, Format: ""})
		if err != nil {
			return "", fmt.Errorf("Invalid format for parameter %s: %w", p.name, err)
		}
	}
	value := func(s *string) string {
		if s == nil {
			return "-"
		}
		return "+" + hex.EncodeToString([]byte(*s))
	}
	exp := "-"
	if expiration != nil {
		exp = strconv.Itoa(*expiration)
	}
	return strings.Join([]string{"OK", value(path), value(username), value(signature), exp}, "\t"), nil
}

func main() {
	if runtime.Version() != "go1.26.8" || strconv.IntSize != 64 {
		panic("Go 1.26.8 / 64-bit int required")
	}
	info, ok := debug.ReadBuildInfo()
	pinned := false
	if ok {
		for _, dep := range info.Deps {
			if dep.Path == "github.com/oapi-codegen/runtime" && dep.Version == "v1.6.0" && dep.Replace == nil {
				pinned = true
			}
		}
	}
	if !pinned {
		panic("original oapi runtime v1.6.0 required")
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		raw, err := io.ReadAll(r.Body)
		if err != nil {
			panic(err)
		}
		if _, err = bind(string(raw)); err != nil {
			http.Error(w, err.Error(), http.StatusBadRequest)
			return
		}
		panic("HTTP mode requires an invalid query")
	}))
	defer server.Close()
	client := server.Client()
	defer client.CloseIdleConnections()
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 65536), 2<<20)
	out := bufio.NewWriter(os.Stdout)
	for scanner.Scan() {
		fields := strings.SplitN(scanner.Text(), "\t", 2)
		if len(fields) != 2 {
			panic("mode and hex query required")
		}
		raw, err := hex.DecodeString(fields[1])
		if err != nil {
			panic(err)
		}
		if fields[0] == "P" {
			result, err := bind(string(raw))
			if err != nil {
				result = "ERR\t" + hex.EncodeToString([]byte(err.Error()+"\n"))
			}
			fmt.Fprintln(out, result)
		} else if fields[0] == "H" {
			response, err := client.Post(server.URL, "application/octet-stream", strings.NewReader(string(raw)))
			if err != nil {
				panic(err)
			}
			data, err := io.ReadAll(response.Body)
			response.Body.Close()
			if err != nil {
				panic(err)
			}
			fmt.Fprintf(out, "%d\t%s\t%s\t%s\t%s\n", response.StatusCode, response.Header.Get("Content-Type"), response.Header.Get("Content-Length"), response.Header.Get("X-Content-Type-Options"), hex.EncodeToString(data))
		} else {
			panic("unknown mode")
		}
		if err := out.Flush(); err != nil {
			panic(err)
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
