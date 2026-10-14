// Original GetAuthUser failure expressions from runtime f455fd58fa3f289e78ce1de690c35968d8d29322
// permissions/authenticate.go, after a controlled missing-default observation.
// Original Connect and generated Start messages run over real HTTP; no process
// or NSS lookup executes. Apache-2.0: process/E2B-LICENSE.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"

	"connectrpc.com/authn"
	"connectrpc.com/connect"
	rpc "sandproof.local/connect-reference/process"
)

const commandDefaultAccountPath = "/fixture.CommandAccount/Start"

func registerDefaultAccounts(mux *http.ServeMux) {
	mux.Handle(commandDefaultAccountPath, connect.NewServerStreamHandler(commandDefaultAccountPath,
		func(_ context.Context, req *connect.Request[rpc.StartRequest], _ *connect.ServerStream[rpc.StartResponse]) error {
			name, err := hex.DecodeString(req.Header().Get("X-Fixture-Username"))
			if err != nil {
				panic(err)
			}
			if len(name) == 0 {
				return connect.NewError(connect.CodeUnauthenticated, errors.New("no user specified"))
			}
			return authn.Errorf("invalid default user: '%s'", name)
		}))
}
func commandDefaultAccountHeaders(req *http.Request, data []byte) {
	var input struct {
		User    string
		Headers map[string][]string
	}
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	req.Header.Set("Content-Type", "application/connect+json")
	req.Header.Set("X-Fixture-Username", hex.EncodeToString([]byte(input.User)))
	for name, values := range input.Headers {
		req.Header.Del(name)
		for _, value := range values {
			req.Header.Add(name, value)
		}
	}
}
