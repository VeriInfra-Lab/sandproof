// Fixed runtime f455fd58fa3f289e78ce1de690c35968d8d29322 auth.go and keys/sha256.go.
// Apache-2.0: ../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
// Original generateSignature/validateSigning bodies below. Only dependency
// names, zeroization helper and an injected Unix clock are adapted. The token
// store/logger are test substitutes; no MMDS, file endpoint or full envd runs.
package main

import (
	"bufio"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"sync/atomic"
	"time"
)

const accessTokenHeader = "X-Access-Token"

type tokenStore string

func (t tokenStore) IsSet() bool { return t != "" }
func (t tokenStore) Bytes() ([]byte, error) {
	if t == "" {
		return nil, errors.New("unset")
	}
	return []byte(t), nil
}
func (t tokenStore) Equals(s string) bool {
	return subtle.ConstantTimeCompare([]byte(t), []byte(s)) == 1
}
func wipeBytes(b []byte) { clear(b) }

type logger struct{}

func (l logger) Error() logger    { return l }
func (l logger) Warn() logger     { return l }
func (l logger) Err(error) logger { return l }
func (logger) Msg(string)         {}

type API struct {
	accessToken tokenStore
	now         int64
	logger      logger
	handover    *struct{}
	initialized atomic.Bool
}

var authExcludedPaths = []string{
	"GET/health",
	"GET/files",
	"POST/files",
	"POST/init",
}

// handoverPreInitAllowedPaths is the MINIMAL set reachable on a live-upgraded
// envd before its post-upgrade /init has restored the access token: only /init
// (which restores auth and lifts this gate, self-authenticated via MMDS) and
// the health check. It deliberately omits /files — unlike authExcludedPaths —
// so a re-adopted (possibly hostile) guest process can't reach the
// root-privileged file API unauthenticated in that window. The orchestrator
// delivers the upgrade over /upgrade's body, not /files, so nothing legitimate
// needs /files before /init.
var handoverPreInitAllowedPaths = []string{
	"GET/health",
	"POST/init",
}

func (a *API) WithAuthorization(handler http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		// check if this path is allowed without authentication (e.g., health check, endpoints supporting signing)
		allowedPath := slices.Contains(authExcludedPaths, req.Method+req.URL.Path)

		switch {
		case a.accessToken.IsSet():
			authHeader := req.Header.Get(accessTokenHeader)

			if !a.accessToken.Equals(authHeader) && !allowedPath {
				a.logger.Error().Msg("Trying to access secured envd without correct access token")

				err := errors.New("unauthorized access, please provide a valid access token or method signing if supported")
				jsonError(w, http.StatusUnauthorized, err)

				return
			}

		case a.handover != nil && !a.initialized.Load() && !slices.Contains(handoverPreInitAllowedPaths, req.Method+req.URL.Path):
			// A live-upgraded envd serves before its post-upgrade /init has
			// restored the access token — and the fallback thaw may already be
			// running the re-adopted workload. The sandbox HAD a token (it is a
			// resume), so treat the unset token as "not yet restored" and fail
			// CLOSED here rather than falling through to the open path below,
			// which would let a re-adopted (and possibly hostile) guest process
			// reach control endpoints unauthenticated in this window. Only the
			// minimal handoverPreInitAllowedPaths (/init, /health) get through —
			// notably NOT /files, whose root file API is otherwise unauthenticated
			// (it is in authExcludedPaths). /init self-authenticates via MMDS and
			// lifts this gate.
			a.logger.Warn().Msg("blocking pre-init request on live-upgraded envd (auth not yet restored)")

			jsonError(w, http.StatusUnauthorized, errors.New("envd not initialized"))

			return
		}

		handler.ServeHTTP(w, req)
	})
}

type Sha256Hashing struct{}

func NewSHA256Hashing() *Sha256Hashing { return &Sha256Hashing{} }
func (h *Sha256Hashing) HashWithoutPrefix(key []byte) string {
	hashBytes := sha256.Sum256(key)
	return base64.RawStdEncoding.EncodeToString(hashBytes[:])
}
func (a *API) generateSignature(path string, username string, operation string, signatureExpiration *int64) (string, error) {
	tokenBytes, err := a.accessToken.Bytes()
	if err != nil {
		return "", fmt.Errorf("access token is not set: %w", err)
	}
	defer wipeBytes(tokenBytes)

	var signature string
	hasher := NewSHA256Hashing()

	if signatureExpiration == nil {
		signature = strings.Join([]string{path, operation, username, string(tokenBytes)}, ":")
	} else {
		signature = strings.Join([]string{path, operation, username, string(tokenBytes), strconv.FormatInt(*signatureExpiration, 10)}, ":")
	}

	return fmt.Sprintf("v1_%s", hasher.HashWithoutPrefix([]byte(signature))), nil
}

func (a *API) validateSigning(r *http.Request, signature *string, signatureExpiration *int, username *string, path string, operation string) (err error) {
	var expectedSignature string

	// no need to validate signing key if access token is not set
	if !a.accessToken.IsSet() {
		return nil
	}

	// check if access token is sent in the header
	tokenFromHeader := r.Header.Get(accessTokenHeader)
	if tokenFromHeader != "" {
		if !a.accessToken.Equals(tokenFromHeader) {
			return errors.New("access token present in header but does not match")
		}

		return nil
	}

	if signature == nil {
		return errors.New("missing signature query parameter")
	}

	// Empty string is used when no username is provided and the default user should be used
	signatureUsername := ""
	if username != nil {
		signatureUsername = *username
	}

	if signatureExpiration == nil {
		expectedSignature, err = a.generateSignature(path, signatureUsername, operation, nil)
	} else {
		exp := int64(*signatureExpiration)
		expectedSignature, err = a.generateSignature(path, signatureUsername, operation, &exp)
	}

	if err != nil {
		a.logger.Error().Err(err).Msg("error generating signing key")

		return errors.New("invalid signature")
	}

	// signature validation
	// Use constant-time comparison to prevent timing attacks.
	if subtle.ConstantTimeCompare([]byte(expectedSignature), []byte(*signature)) != 1 {
		return errors.New("invalid signature")
	}

	// signature expiration
	if signatureExpiration != nil {
		exp := int64(*signatureExpiration)
		if exp < a.now {
			return errors.New("signature is already expired")
		}
	}

	return nil
}

func unhex(s string) string {
	b, e := hex.DecodeString(s)
	if e != nil {
		panic(e)
	}
	return string(b)
}
func optional(s string) *string {
	if s == "-" {
		return nil
	}
	v := unhex(strings.TrimPrefix(s, "+"))
	return &v
}
func expiration(s string) *int {
	if s == "-" {
		return nil
	}
	v, e := strconv.ParseInt(s, 10, 64)
	if e != nil {
		panic(e)
	}
	n := int(v)
	return &n
}

// Original api/error.go jsonError, using the same generated field types.
// The transport observes its real GET/HEAD/POST wire behavior; authentication
// below still comes from original validateSigning, not an injected message.
type Error struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

func jsonError(w http.ResponseWriter, code int, err error) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")
	w.WriteHeader(code)
	encodeErr := json.NewEncoder(w).Encode(Error{Code: code, Message: err.Error()})
	if encodeErr != nil {
		http.Error(w, errors.Join(encodeErr, err).Error(), http.StatusInternalServerError)
	}
}

// RPC mode exercises the original outer middleware over real HTTP with a
// controlled initialized token store; no MMDS/account/filesystem code runs.
func rpcAuthorizationHTTP(data []byte) string {
	var input struct {
		Method  string
		Path    string
		Headers []string
	}
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	a := API{accessToken: tokenStore("token")}
	server := httptest.NewServer(a.WithAuthorization(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		panic("reference token rejection reached protected handler")
	})))
	defer server.Close()
	if input.Path == "" {
		input.Path = "/filesystem.Filesystem/MakeDir"
	}
	req, err := http.NewRequest(input.Method, server.URL+input.Path, strings.NewReader("invalid JSON"))
	if err != nil {
		panic(err)
	}
	for _, value := range input.Headers {
		req.Header.Add(accessTokenHeader, unhex(value))
	}
	req.Header.Set("Authorization", "Basic bWlzc2luZzo=")
	req.Header.Set("Content-Type", "application/grpc")
	req.Header.Set("Content-Encoding", "br")
	req.Header.Set("Accept-Encoding", "identity")
	req.Header.Set("Connect-Protocol-Version", "bad")
	req.Header.Set("Connect-Timeout-Ms", "bad")
	client := server.Client()
	client.Timeout = 5 * time.Second
	res, err := client.Do(req)
	if err != nil {
		panic(err)
	}
	body, err := io.ReadAll(res.Body)
	res.Body.Close()
	if err != nil {
		panic(err)
	}
	if res.StatusCode != http.StatusUnauthorized {
		panic("expected outer token rejection")
	}
	values := []string{strconv.Itoa(res.StatusCode)}
	for _, name := range []string{"Content-Type", "Content-Length", "X-Content-Type-Options", "Accept-Encoding", "Content-Encoding"} {
		values = append(values, hex.EncodeToString([]byte(res.Header.Get(name))))
	}
	return strings.Join(append(values, hex.EncodeToString(body)), "\t")
}

func main() {
	if runtime.Version() != "go1.26.8" || strconv.IntSize != 64 {
		panic("Go 1.26.8, 64-bit int required")
	}
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 65536), 2<<20)
	out := bufio.NewWriter(os.Stdout)
	var server *httptest.Server
	defer func() {
		if server != nil {
			server.Close()
		}
	}()
	for scanner.Scan() {
		if input, ok := strings.CutPrefix(scanner.Text(), "R\t"); ok {
			fmt.Fprintln(out, rpcAuthorizationHTTP([]byte(unhex(input))))
			if err := out.Flush(); err != nil {
				panic(err)
			}
			continue
		}
		f := strings.Split(scanner.Text(), "\t")
		if len(f) < 8 {
			panic("mode, token, path, username, expiration, operation, signature, now[,headers...] required")
		}
		a := API{accessToken: tokenStore(unhex(f[1]))}
		a.now, _ = strconv.ParseInt(f[7], 10, 64)
		path, user, exp, op := unhex(f[2]), optional(f[3]), expiration(f[4]), f[5]
		if f[0] == "S" {
			username := ""
			if user != nil {
				username = *user
			}
			var deadline *int64
			if exp != nil {
				n := int64(*exp)
				deadline = &n
			}
			value, err := a.generateSignature(path, username, op, deadline)
			if err != nil {
				panic(err)
			}
			fmt.Fprintln(out, value)
		} else if f[0] == "V" || strings.HasPrefix(f[0], "H") {
			request := &http.Request{Header: make(http.Header)}
			for _, header := range f[8:] {
				request.Header.Add(accessTokenHeader, unhex(header))
			}
			err := a.validateSigning(request, optional(f[6]), exp, user, path, op)
			if f[0] != "V" {
				if err == nil {
					panic("HTTP mode requires actual signing denial")
				}
				if server == nil {
					server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
						message, failure := io.ReadAll(r.Body)
						if failure != nil {
							panic(failure)
						}
						jsonError(w, http.StatusUnauthorized, errors.New(string(message)))
					}))
				}
				r, failure := http.NewRequest(strings.TrimPrefix(f[0], "H"), server.URL, strings.NewReader(err.Error()))
				if failure != nil {
					panic(failure)
				}
				client := server.Client()
				client.Timeout = 5 * time.Second
				response, failure := client.Do(r)
				if failure != nil {
					panic(failure)
				}
				data, failure := io.ReadAll(response.Body)
				response.Body.Close()
				if failure != nil {
					panic(failure)
				}
				fmt.Fprintf(out, "%d\t%x\t%x\t%x\t%x\n", response.StatusCode, response.Header.Get("Content-Type"), response.Header.Get("Content-Length"), response.Header.Get("X-Content-Type-Options"), data)
			} else if err == nil {
				fmt.Fprintln(out, "OK")
			} else {
				fmt.Fprintln(out, "ERR\t"+hex.EncodeToString([]byte(err.Error())))
			}
		} else {
			panic("invalid mode")
		}
		if err := out.Flush(); err != nil {
			panic(err)
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
