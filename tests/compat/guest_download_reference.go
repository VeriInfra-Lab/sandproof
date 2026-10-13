// Go 1.26.8 net/http.ParseTime and ServeContent oracle. Uses the original
// standard-library functions, synthetic bytes and httptest, not a live envd,
// filesystem, SDK or product VM. Local-zone input is explicit per test.
package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"fmt"
	"mime"
	"net/http"
	"net/http/httptest"
	"os"
	"runtime"
	"strconv"
	"strings"
	"time"
)

func decode(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}
func integer(s string) int64 {
	n, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		panic(err)
	}
	return n
}
func local(name string, offset string) {
	time.Local = time.UTC
	if name != "" {
		time.Local = time.FixedZone(string(decode(name)), int(integer(offset)))
	}
}
func main() {
	if runtime.Version() != "go1.26.8" {
		panic("reference requires go1.26.8")
	}
	if err := os.Setenv("GODEBUG", "httpservecontentkeepheaders=0"); err != nil {
		panic(err)
	}
	input := bufio.NewScanner(os.Stdin)
	input.Buffer(make([]byte, 4096), 1<<20)
	for input.Scan() {
		f := strings.Split(input.Text(), "\t")
		switch f[0] {
		case "T":
			if len(f) != 4 {
				panic("date fields")
			}
			local(f[2], f[3])
			t, err := http.ParseTime(string(decode(f[1])))
			if err != nil {
				fmt.Println("ERR")
			} else {
				fmt.Printf("%d\t%d\n", t.Unix(), t.Nanosecond())
			}
		case "P":
			if len(f) != 14 {
				panic("response fields")
			}
			local(f[12], f[13])
			size := integer(f[1])
			if size < 0 || size > 8192 {
				panic("fixture size")
			}
			data := make([]byte, size)
			for i := range data {
				data[i] = byte(i % 251)
			}
			request := &http.Request{Method: f[4], Header: make(http.Header)}
			for i, name := range []string{"If-Match", "If-Unmodified-Since", "If-None-Match", "If-Modified-Since", "If-Range", "Range"} {
				request.Header.Set(name, string(decode(f[6+i])))
			}
			result := httptest.NewRecorder()
			for name, value := range map[string]string{"Content-Type": "application/octet-stream", "Vary": "Accept-Encoding", "Content-Disposition": "inline", "Cache-Control": "private"} {
				result.Header().Set(name, value)
			}
			if f[5] != "" {
				result.Header().Set("ETag", string(decode(f[5])))
			}
			http.ServeContent(result, request, "fixture.bin", time.Unix(integer(f[2]), integer(f[3])), bytes.NewReader(data))
			response := result.Result()
			body := result.Body.Bytes()
			ctype, params, _ := mime.ParseMediaType(response.Header.Get("Content-Type"))
			if ctype == "multipart/byteranges" {
				boundary := params["boundary"]
				if len(boundary) != 60 {
					panic("unexpected Go boundary length")
				}
				normalized := strings.Repeat("a", 60)
				body = bytes.ReplaceAll(body, []byte(boundary), []byte(normalized))
				response.Header.Set("Content-Type", "multipart/byteranges; boundary="+normalized)
			}
			fmt.Printf("%d", response.StatusCode)
			for _, name := range []string{"Last-Modified", "Content-Range", "Accept-Ranges", "Content-Type", "Content-Length", "ETag", "Cache-Control", "Content-Encoding", "X-Content-Type-Options", "Vary", "Content-Disposition"} {
				fmt.Printf("\t%x", response.Header.Get(name))
			}
			fmt.Printf("\t%x\n", body)
			response.Body.Close()
		default:
			panic("unknown operation")
		}
	}
	if err := input.Err(); err != nil {
		panic(err)
	}
}
