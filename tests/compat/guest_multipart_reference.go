// Component oracle: Go 1.26.8 standard MIME reader, used by the fixed envd
// revision. This does not call envd endpoints, open files or start a listener.
// Build with the fixed Go version; run the ignored Rust reference test explicitly.
package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"fmt"
	"io"
	"mime"
	"mime/multipart"
	"os"
	"runtime"
	"strings"
)

func main() {
	if runtime.Version() != "go1.26.8" {
		panic("reference requires go1.26.8")
	}
	input := bufio.NewScanner(os.Stdin)
	input.Buffer(make([]byte, 4096), 4<<20)
	for input.Scan() {
		fields := strings.Split(input.Text(), "\t")
		if len(fields) != 2 {
			panic("expected boundary and body as hex")
		}
		boundary, err := hex.DecodeString(fields[0])
		if err != nil {
			panic(err)
		}
		body, err := hex.DecodeString(fields[1])
		if err != nil {
			panic(err)
		}
		reader := multipart.NewReader(bytes.NewReader(body), string(boundary))
		var result strings.Builder
		for {
			part, err := reader.NextPart()
			if err == io.EOF {
				result.WriteString("END")
				break
			}
			if err != nil {
				result.WriteString("ERROR")
				break
			}
			_, params, _ := mime.ParseMediaType(part.Header.Get("Content-Disposition"))
			filename := "-"
			if value, ok := params["filename"]; ok {
				filename = "+" + hex.EncodeToString([]byte(value))
			}
			data, err := io.ReadAll(part)
			fmt.Fprintf(&result, "P:%x:%s:%x:%t;", part.FormName(), filename, data, err == nil)
			if err != nil {
				result.WriteString("ERROR")
				break
			}
		}
		fmt.Println(result.String())
	}
	if err := input.Err(); err != nil {
		panic(err)
	}
}
