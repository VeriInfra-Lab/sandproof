// Original authn/Connect ErrorWriter, with a controlled missing-account result.
// This observes middleware HTTP behavior; it does not replace an NSS lookup.
package main

import (
	"connectrpc.com/authn"
	"context"
	"encoding/json"
	"net/http"
)

const commandAccountPath = "/fixture.CommandAccount/Reject"

func registerCommandAccounts(mux *http.ServeMux) {
	mux.Handle(commandAccountPath, authn.NewMiddleware(func(_ context.Context, req authn.Request) (any, error) {
		user, _, ok := req.BasicAuth()
		if !ok {
			panic("missing original Basic input")
		}
		return nil, authn.Errorf("invalid username: '%s'", user)
	}).Wrap(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { panic("missing user reached handler") })))
}
func commandAccountHeaders(req *http.Request, data []byte) {
	var input struct {
		Method, User string
		Headers      map[string][]string
	}
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	req.Method = input.Method
	req.Header = make(http.Header)
	req.SetBasicAuth(input.User, "ignored-private-password")
	for name, values := range input.Headers {
		for _, value := range values {
			req.Header.Add(name, value)
		}
	}
}
