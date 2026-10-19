// HTTP method selection by the original unary handler, before codec/body work.
package main

import (
	"encoding/json"
	"net/http"
)

type methodInput struct {
	RPC, Method string
	ContentType []string
}

func methodEndpoint(data []byte) (string, []byte) {
	var input methodInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	switch input.RPC {
	case "Stat", "MakeDir", "ListDir", "Move", "Remove":
	default:
		panic("unknown unary method reference")
	}
	return "/fixture.Requests/" + input.RPC, []byte("invalid JSON")
}

func methodHeaders(req *http.Request, data []byte) {
	var input methodInput
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	req.Method = input.Method
	req.Header.Del("Content-Type")
	for _, value := range input.ContentType {
		req.Header.Add("Content-Type", value)
	}
	// All of these would fail later stages if method selection did not win.
	req.Header.Set("Content-Encoding", "br")
	req.Header.Set("Connect-Protocol-Version", "bad")
	req.Header.Set("Connect-Timeout-Ms", "bad")
}
