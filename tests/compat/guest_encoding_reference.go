// Fixed envd encoding oracle; only the package/imports and driver are changed.
// Selection functions below are copied verbatim from e2b-dev/runtime commit
// f455fd58fa3f289e78ce1de690c35968d8d29322 packages/envd/internal/api/encoding.go.
// Apache-2.0: ../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
// The service, HTTP effects and filesystem are NOT exercised here.
package main

import (
	"bufio"
	"cmp"
	"encoding/hex"
	"fmt"
	"math"
	"net/http"
	"os"
	"runtime"
	"slices"
	"strconv"
	"strings"
)

const (
	// EncodingGzip is the gzip content encoding.
	EncodingGzip = "gzip"
	// EncodingIdentity means no encoding (passthrough).
	EncodingIdentity = "identity"
	// EncodingWildcard means any encoding is acceptable.
	EncodingWildcard = "*"
)

// SupportedEncodings lists the content encodings supported for file transfer.
// The order matters - encodings are checked in order of preference.
var SupportedEncodings = []string{
	EncodingGzip,
}

// encodingWithQuality holds an encoding name and its quality value.
type encodingWithQuality struct {
	encoding string
	quality  float64
}

// isSupportedEncoding checks if the given encoding is in the supported list.
// Per RFC 7231, content-coding values are case-insensitive.
func isSupportedEncoding(encoding string) bool {
	return slices.Contains(SupportedEncodings, strings.ToLower(encoding))
}

// parseEncodingWithQuality parses an encoding value and extracts the quality.
// Returns the encoding name (lowercased) and quality value (default 1.0 if not specified).
// Per RFC 7231, content-coding values are case-insensitive.
func parseEncodingWithQuality(value string) encodingWithQuality {
	value = strings.TrimSpace(value)
	quality := 1.0

	if idx := strings.Index(value, ";"); idx != -1 {
		params := value[idx+1:]
		value = strings.TrimSpace(value[:idx])

		// Parse q=X.X parameter
		for param := range strings.SplitSeq(params, ";") {
			param = strings.TrimSpace(param)
			if strings.HasPrefix(strings.ToLower(param), "q=") {
				if q, err := strconv.ParseFloat(param[2:], 64); err == nil {
					quality = q
				}
			}
		}
	}

	// Normalize encoding to lowercase per RFC 7231
	return encodingWithQuality{encoding: strings.ToLower(value), quality: quality}
}

// parseEncoding extracts the encoding name from a header value, stripping quality.
func parseEncoding(value string) string {
	return parseEncodingWithQuality(value).encoding
}

// parseContentEncoding parses the Content-Encoding header and returns the encoding.
// Returns an error if an unsupported encoding is specified.
// If no Content-Encoding header is present, returns empty string.
func parseContentEncoding(r *http.Request) (string, error) {
	header := r.Header.Get("Content-Encoding")
	if header == "" {
		return EncodingIdentity, nil
	}

	encoding := parseEncoding(header)

	if encoding == EncodingIdentity {
		return EncodingIdentity, nil
	}

	if !isSupportedEncoding(encoding) {
		return "", fmt.Errorf("unsupported Content-Encoding: %s, supported: %v", header, SupportedEncodings)
	}

	return encoding, nil
}

// parseAcceptEncodingHeader parses the Accept-Encoding header and returns
// the parsed encodings along with the identity rejection state.
// Per RFC 7231 Section 5.3.4, identity is acceptable unless excluded by
// "identity;q=0" or "*;q=0" without a more specific entry for identity with q>0.
func parseAcceptEncodingHeader(header string) ([]encodingWithQuality, bool) {
	if header == "" {
		return nil, false // identity not rejected when header is empty
	}

	// Parse all encodings with their quality values
	var encodings []encodingWithQuality
	for value := range strings.SplitSeq(header, ",") {
		eq := parseEncodingWithQuality(value)
		encodings = append(encodings, eq)
	}

	// Check if identity is rejected per RFC 7231 Section 5.3.4:
	// identity is acceptable unless excluded by "identity;q=0" or "*;q=0"
	// without a more specific entry for identity with q>0.
	identityRejected := false
	identityExplicitlyAccepted := false
	wildcardRejected := false

	for _, eq := range encodings {
		switch eq.encoding {
		case EncodingIdentity:
			if eq.quality == 0 {
				identityRejected = true
			} else {
				identityExplicitlyAccepted = true
			}
		case EncodingWildcard:
			if eq.quality == 0 {
				wildcardRejected = true
			}
		}
	}

	if wildcardRejected && !identityExplicitlyAccepted {
		identityRejected = true
	}

	return encodings, identityRejected
}

// isIdentityAcceptable checks if identity encoding is acceptable based on the
// Accept-Encoding header. Per RFC 7231 section 5.3.4, identity is always
// implicitly acceptable unless explicitly rejected with q=0.
func isIdentityAcceptable(r *http.Request) bool {
	header := r.Header.Get("Accept-Encoding")
	_, identityRejected := parseAcceptEncodingHeader(header)

	return !identityRejected
}

// parseAcceptEncoding parses the Accept-Encoding header and returns the best
// supported encoding based on quality values. Per RFC 7231 section 5.3.4,
// identity is always implicitly acceptable unless explicitly rejected with q=0.
// If no Accept-Encoding header is present, returns empty string (identity).
func parseAcceptEncoding(r *http.Request) (string, error) {
	header := r.Header.Get("Accept-Encoding")
	if header == "" {
		return EncodingIdentity, nil
	}

	encodings, identityRejected := parseAcceptEncodingHeader(header)

	// Sort by quality value (highest first)
	slices.SortFunc(encodings, func(a, b encodingWithQuality) int {
		return cmp.Compare(b.quality, a.quality)
	})

	// Find the best supported encoding
	for _, eq := range encodings {
		// Skip encodings with q=0 (explicitly rejected)
		if eq.quality == 0 {
			continue
		}

		if eq.encoding == EncodingIdentity {
			return EncodingIdentity, nil
		}

		// Wildcard means any encoding is acceptable - return a supported encoding if identity is rejected
		if eq.encoding == EncodingWildcard {
			if identityRejected && len(SupportedEncodings) > 0 {
				return SupportedEncodings[0], nil
			}

			return EncodingIdentity, nil
		}

		if isSupportedEncoding(eq.encoding) {
			return eq.encoding, nil
		}
	}

	// Per RFC 7231, identity is implicitly acceptable unless rejected
	if !identityRejected {
		return EncodingIdentity, nil
	}

	// Identity rejected and no supported encodings found
	return "", fmt.Errorf("no acceptable encoding found, supported: %v", SupportedEncodings)
}

func main() {
	if runtime.Version() != "go1.26.8" {
		panic("reference requires go1.26.8")
	}
	input := bufio.NewScanner(os.Stdin)
	input.Buffer(make([]byte, 4096), 1<<20)
	for input.Scan() {
		fields := strings.Split(input.Text(), "\t")
		if len(fields) != 2 {
			panic("expected operation and hex header")
		}
		header, err := hex.DecodeString(fields[1])
		if err != nil {
			panic(err)
		}
		if fields[0] == "Q" {
			q, err := strconv.ParseFloat(string(header), 64)
			if err != nil {
				fmt.Println("ERR")
			} else if math.IsNaN(q) {
				fmt.Println("NAN")
			} else {
				fmt.Printf("%016x\n", math.Float64bits(q))
			}
			continue
		}
		r := &http.Request{Header: make(http.Header)}
		r.Header.Add("Content-Encoding", string(header))
		r.Header.Add("Accept-Encoding", string(header))
		var encoding string
		switch fields[0] {
		case "C":
			encoding, err = parseContentEncoding(r)
		case "A":
			encoding, err = parseAcceptEncoding(r)
		case "D":
			encoding, err = parseAcceptEncoding(r)
			if err == nil {
				if !isIdentityAcceptable(r) {
					err = fmt.Errorf("identity rejected for conditional request")
				} else {
					encoding = EncodingIdentity
				}
			}
		default:
			panic("unknown operation")
		}
		if err != nil {
			fmt.Println("ERR")
		} else {
			fmt.Println(encoding)
		}
	}
	if err := input.Err(); err != nil {
		panic(err)
	}
}
