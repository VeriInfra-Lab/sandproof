// Original Connect JSON header negotiation on controlled process handlers.
// These handlers do not execute commands or prove that an expired context
// prevents user effects; only the HTTP/codec/metadata observations are compared.
package main

import (
	"bytes"
	"compress/gzip"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"strings"
)

type commandHeaderInput struct {
	RPC, Method string
	Headers     map[string][]string
}

func commandHeaderEndpoint(data []byte) (string, []byte) {
	var input commandHeaderInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	switch input.RPC {
	case "Start", "Connect":
		return "/fixture.Process/" + input.RPC, []byte{0, 0, 0, 0, 2, '{', '}'}
	case "List", "SendInput", "SendSignal", "CloseStdin", "Update":
		body := []byte("{}")
		if values := input.Headers["content-encoding"]; len(values) > 0 && values[0] == "gzip" {
			var compressed bytes.Buffer
			writer := gzip.NewWriter(&compressed)
			if _, err := writer.Write(body); err != nil {
				panic(err)
			}
			if err := writer.Close(); err != nil {
				panic(err)
			}
			body = compressed.Bytes()
		}
		return "/fixture.Process/" + input.RPC, body
	default:
		panic("unknown command header method")
	}
}
func commandHeaders(req *http.Request, data []byte) {
	var input commandHeaderInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	req.Method = input.Method
	if input.RPC == "Start" || input.RPC == "Connect" {
		req.Header.Set("Content-Type", "application/connect+json")
	}
	req.Header.Set("X-Fixture-Command", hex.EncodeToString([]byte(`{"Event":{"Kind":"Start","PID":123}}`)))
	for name, values := range input.Headers {
		req.Header.Del(name)
		for _, value := range values {
			req.Header.Add(name, value)
		}
	}
}
func commandHeaderObservation(res *http.Response, body []byte) string {
	result := struct {
		Status      int    `json:"status"`
		ContentType string `json:"contentType"`
		Encoding    string `json:"encoding"`
		Code        string `json:"code"`
		Message     string `json:"message"`
		Allow       string `json:"allow"`
	}{Status: res.StatusCode, ContentType: res.Header.Get("Content-Type"), Allow: res.Header.Get("Allow")}
	stream := len(result.ContentType) >= 20 && result.ContentType[:20] == "application/connect+"
	name := "Content-Encoding"
	if stream {
		name = "Connect-Content-Encoding"
	}
	result.Encoding = res.Header.Get(name)
	if !stream && result.Encoding == "gzip" {
		body = commandHeaderExpand(body)
	}
	if stream {
		for len(body) > 0 {
			if len(body) < 5 {
				panic("short original frame")
			}
			n := int(binary.BigEndian.Uint32(body[1:5]))
			if n > len(body)-5 {
				panic("truncated original frame")
			}
			flags, payload := body[0], body[5:5+n]
			body = body[5+n:]
			if flags&1 != 0 {
				payload = commandHeaderExpand(payload)
			}
			if flags&2 != 0 {
				var end struct {
					Error *struct{ Code, Message string }
				}
				if err := json.Unmarshal(payload, &end); err != nil {
					panic(err)
				}
				if end.Error != nil {
					result.Code = end.Error.Code
					result.Message = end.Error.Message
				}
			}
		}
	} else if len(body) > 0 && res.StatusCode >= 400 {
		var failure struct{ Code, Message string }
		if err := json.Unmarshal(body, &failure); err != nil {
			panic(err)
		}
		result.Code = failure.Code
		result.Message = failure.Message
	}
	// Normalize only this outer observation container, after decoding the
	// original HTTP error/frame. Its message string remains unchanged.
	var output bytes.Buffer
	encoder := json.NewEncoder(&output)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(result); err != nil {
		panic(err)
	}
	return strings.TrimSuffix(output.String(), "\n")
}
func commandHeaderExpand(body []byte) []byte {
	reader, err := gzip.NewReader(bytes.NewReader(body))
	if err != nil {
		panic(err)
	}
	data, err := io.ReadAll(reader)
	if err != nil {
		panic(err)
	}
	if err := reader.Close(); err != nil {
		panic(err)
	}
	return data
}
