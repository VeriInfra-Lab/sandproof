// Fixed Go 1.26.8 upload header oracle; original mime.ParseMediaType and
// fixed envd extraction/validation (f455fd58fa3f289e78ce1de690c35968d8d29322).
// Apache-2.0: ../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
package main

import (
	"bufio"
	"encoding/hex"
	"errors"
	"fmt"
	"mime"
	"net/http"
	"os"
	"runtime"
	"sort"
	"strings"
)

const MetadataXattrPrefix = "user.e2b."
const MaxMetadataKeyLen = 255 - len(MetadataXattrPrefix)
const MaxMetadataTotalLen = 4096

func ValidateMetadata(metadata map[string]string) error {
	total := 0
	for k, v := range metadata {
		if k == "" {
			return errors.New("metadata key must not be empty")
		}
		if len(k) > MaxMetadataKeyLen {
			return fmt.Errorf("metadata key %q exceeds %d bytes", k, MaxMetadataKeyLen)
		}
		if err := validatePrintableASCII("key", k); err != nil {
			return err
		}
		if err := validatePrintableASCII(fmt.Sprintf("value for key %q", k), v); err != nil {
			return err
		}
		total += len(MetadataXattrPrefix) + len(k) + len(v)
	}

	if total > MaxMetadataTotalLen {
		return fmt.Errorf("total metadata size %d exceeds %d bytes", total, MaxMetadataTotalLen)
	}

	return nil
}

func validatePrintableASCII(label, s string) error {
	for i := range len(s) {
		c := s[i]
		if c < 0x20 || c > 0x7E {
			return fmt.Errorf("metadata %s contains non-printable-ASCII byte 0x%02x at offset %d", label, c, i)
		}
	}

	return nil
}

const metadataHeaderPrefix = "X-Metadata-"

func extractMetadataHeaders(h http.Header) map[string]string {
	var metadata map[string]string
	for name, values := range h {
		if len(values) == 0 || !strings.HasPrefix(name, metadataHeaderPrefix) {
			continue
		}
		key := strings.ToLower(strings.TrimPrefix(name, metadataHeaderPrefix))
		if key == "" {
			continue
		}
		if metadata == nil {
			metadata = make(map[string]string)
		}
		metadata[key] = values[0]
	}
	return metadata
}
func unhex(v string) []byte {
	b, e := hex.DecodeString(v)
	if e != nil {
		panic(e)
	}
	return b
}
func main() {
	if runtime.Version() != "go1.26.8" {
		panic("requires go1.26.8")
	}
	input := bufio.NewScanner(os.Stdin)
	input.Buffer(make([]byte, 4096), 1<<20)
	for input.Scan() {
		f := strings.Split(input.Text(), "\t")
		switch f[0] {
		case "C":
			kind, _, _ := mime.ParseMediaType(string(unhex(f[1])))
			if kind == "application/octet-stream" {
				fmt.Println("R")
			} else if strings.HasPrefix(kind, "multipart/") {
				fmt.Println("M")
			} else {
				fmt.Println("U")
			}
		case "M":
			h := make(http.Header)
			for _, field := range f[1:] {
				kv := strings.SplitN(field, ":", 2)
				h.Add(string(unhex(kv[0])), string(unhex(kv[1])))
			}
			metadata := extractMetadataHeaders(h)
			if ValidateMetadata(metadata) != nil {
				fmt.Println("ERR")
				continue
			}
			keys := make([]string, 0, len(metadata))
			for key := range metadata {
				keys = append(keys, key)
			}
			sort.Strings(keys)
			fmt.Print("OK")
			for _, key := range keys {
				fmt.Printf("\t%x:%x", key, metadata[key])
			}
			fmt.Println()
		default:
			panic("operation")
		}
	}
	if input.Err() != nil {
		panic(input.Err())
	}
}
