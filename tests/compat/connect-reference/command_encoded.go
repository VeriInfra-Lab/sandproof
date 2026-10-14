// Full encoded command requests use the original stream/unary echo handlers.
package main

import (
	"encoding/hex"
	"encoding/json"
	"net/http"
	"strings"
)

type encodedCommandInput struct {
	Method, Body string
	Gzip         bool
}

func encodedCommandRecord(data []byte) encodedCommandInput {
	var input encodedCommandInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	return input
}
func encodedCommandEndpoint(data []byte) (string, []byte) {
	input := encodedCommandRecord(data)
	if input.Method == "Start" || input.Method == "Connect" {
		return framedCommandRequestEndpoint(data)
	}
	return commandRequestEndpoint(data)
}
func encodedCommandHeaders(req *http.Request, data []byte) {
	input := encodedCommandRecord(data)
	name := "Content-Encoding"
	if input.Method == "Start" || input.Method == "Connect" {
		req.Header.Set("Content-Type", "application/connect+json")
		name = "Connect-Content-Encoding"
	}
	if input.Gzip {
		req.Header.Set(name, "gzip")
	}
}
func encodedCommandObservation(res *http.Response, body []byte) string {
	if strings.HasPrefix(res.Header.Get("Content-Type"), "application/connect+") {
		return framedCommandRequestObservation(res, body)
	}
	if res.Header.Get("Content-Encoding") == "gzip" {
		body = commandHeaderExpand(body)
	}
	if res.StatusCode == http.StatusOK {
		return "OK\t" + hex.EncodeToString(body)
	}
	var failure struct{ Code string }
	if err := json.Unmarshal(body, &failure); err != nil || failure.Code == "" {
		panic("unclassified original encoded request failure")
	}
	return "ERR\t" + failure.Code
}
