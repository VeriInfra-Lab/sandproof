// Component oracle for the gzip reader/writer used by fixed envd. No HTTP,
// file effects or product service. Build and run explicitly with Go 1.26.8.
package main

import (
	"bufio"
	"bytes"
	"compress/gzip"
	"encoding/hex"
	"fmt"
	"io"
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
			panic("expected operation and hex body")
		}
		body, err := hex.DecodeString(fields[1])
		if err != nil {
			panic(err)
		}
		switch fields[0] {
		case "D":
			reader, err := gzip.NewReader(bytes.NewReader(body))
			if err != nil {
				fmt.Println("INIT_ERROR\t\tERROR")
				continue
			}
			data, err := io.ReadAll(reader)
			state := "END"
			if err != nil {
				state = "ERROR"
			}
			fmt.Printf("READY\t%x\t%s\n", data, state)
			reader.Close()
		case "E":
			var output bytes.Buffer
			writer := gzip.NewWriter(&output)
			if _, err := writer.Write(body); err != nil {
				panic(err)
			}
			if err := writer.Close(); err != nil {
				panic(err)
			}
			fmt.Printf("%x\n", output.Bytes())
		default:
			panic("unknown operation")
		}
	}
	if err := input.Err(); err != nil {
		panic(err)
	}
}
