// Observe original successful unary/stream headers and decoded messages. Gzip
// bitstreams and insignificant protobuf JSON whitespace are not compared.
package main

import (
	"encoding/binary"
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
)

func commandSuccessHeaders(req *http.Request, data []byte, stream bool) {
	var input struct{ Gzip, UTF8 bool }
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	if input.UTF8 {
		req.Header.Set("Content-Type", req.Header.Get("Content-Type")+"; charset=utf-8")
	}
	if input.Gzip {
		name := "Accept-Encoding"
		if stream {
			name = "Connect-Accept-Encoding"
		}
		req.Header.Set(name, "gzip")
	}
}

func commandSuccessObservation(res *http.Response, body []byte) string {
	headers := make(map[string]string)
	for _, name := range []string{"content-type", "content-encoding", "connect-content-encoding", "accept-encoding", "connect-accept-encoding", "content-length", "vary"} {
		headers[name] = res.Header.Get(name)
	}
	if headers["content-length"] != "" {
		if headers["content-length"] != strconv.Itoa(len(body)) {
			panic("invalid original success length")
		}
		headers["content-length"] = "valid"
	}
	frames := make([]map[string]any, 0)
	add := func(flag byte, payload []byte) {
		var value any
		if err := json.Unmarshal(payload, &value); err != nil {
			panic(err)
		}
		frames = append(frames, map[string]any{"Flags": flag, "Payload": value})
	}
	if strings.HasPrefix(headers["content-type"], "application/connect+") {
		for len(body) != 0 {
			if len(body) < 5 {
				panic("truncated original stream header")
			}
			size := int(binary.BigEndian.Uint32(body[1:5]))
			if size > len(body)-5 {
				panic("truncated original stream message")
			}
			payload := body[5 : 5+size]
			if body[0]&1 != 0 {
				payload = commandHeaderExpand(payload)
			}
			add(body[0], payload)
			body = body[5+size:]
		}
	} else {
		if headers["content-encoding"] == "gzip" {
			body = commandHeaderExpand(body)
		}
		add(0, body)
	}
	result, err := json.Marshal(map[string]any{"Status": res.StatusCode, "Headers": headers, "Frames": frames})
	if err != nil {
		panic(err)
	}
	return string(result)
}
