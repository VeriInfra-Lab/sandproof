// Go 1.26.8 net/http.ParseTime and ServeContent oracle. Uses the original
// standard-library functions, synthetic bytes and httptest, not a live envd,
// filesystem, SDK or product VM. Local-zone input is explicit per test.
package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"mime"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

func decode(s string) []byte {
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return b
}
func integer(s string) int64 {
	n, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		panic(err)
	}
	return n
}
func local(name string, offset string) {
	time.Local = time.UTC
	if name != "" {
		time.Local = time.FixedZone(string(decode(name)), int(integer(offset)))
	}
}

type failingSeeker struct {
	*bytes.Reader
	calls, fail int
	failure     error
}

func (r *failingSeeker) Seek(offset int64, whence int) (int64, error) {
	r.calls++
	if r.calls == r.fail {
		if r.failure != nil {
			return 0, r.failure
		}
		return 0, errors.New("private seek details")
	}
	return r.Reader.Seek(offset, whence)
}
func dump(result *httptest.ResponseRecorder) {
	response := result.Result()
	body := result.Body.Bytes()
	fmt.Printf("%d", response.StatusCode)
	for _, name := range []string{"Last-Modified", "Content-Range", "Accept-Ranges", "Content-Type", "Content-Length", "ETag", "Cache-Control", "Content-Encoding", "X-Content-Type-Options", "Vary", "Content-Disposition"} {
		fmt.Printf("\t%x", response.Header.Get(name))
	}
	fmt.Printf("\t%x\n", body)
	response.Body.Close()
}
func main() {
	if runtime.Version() != "go1.26.8" {
		panic("reference requires go1.26.8")
	}
	if err := os.Setenv("GODEBUG", "httpservecontentkeepheaders=0"); err != nil {
		panic(err)
	}
	input := bufio.NewScanner(os.Stdin)
	input.Buffer(make([]byte, 4096), 1<<20)
	for input.Scan() {
		f := strings.Split(input.Text(), "\t")
		switch f[0] {
		case "T":
			if len(f) != 4 {
				panic("date fields")
			}
			local(f[2], f[3])
			t, err := http.ParseTime(string(decode(f[1])))
			if err != nil {
				fmt.Println("ERR")
			} else {
				fmt.Printf("%d\t%d\n", t.Unix(), t.Nanosecond())
			}
		case "F":
			if len(f) != 4 {
				panic("fault fields")
			}
			request := &http.Request{Method: f[3], Header: make(http.Header)}
			result := httptest.NewRecorder()
			for name, value := range map[string]string{"Vary": "Accept-Encoding", "Content-Disposition": "inline", "ETag": "tag", "Cache-Control": "private", "Content-Encoding": "gzip", "Content-Length": "999"} {
				result.Header().Set(name, value)
			}
			if f[2] == "known" {
				result.Header().Set("Content-Type", "application/octet-stream")
			}
			source := &failingSeeker{Reader: bytes.NewReader([]byte("01234")), fail: int(integer(f[1]))}
			http.ServeContent(result, request, "no-extension", time.Unix(1, 0), source)
			// httptest captures attempted HEAD writes; real net/http suppresses them.
			if f[3] == "HEAD" {
				result.Body.Reset()
			}
			dump(result)
		case "R":
			if len(f) != 4 {
				panic("range fault fields")
			}
			request := &http.Request{Method: f[3], Header: make(http.Header)}
			request.Header.Set("Range", "bytes=1-2")
			result := httptest.NewRecorder()
			for name, value := range map[string]string{"Vary": "Accept-Encoding", "Content-Disposition": "inline", "ETag": "tag", "Cache-Control": "private", "Content-Encoding": "gzip", "Content-Length": "999"} {
				result.Header().Set(name, value)
			}
			result.Header().Set("Content-Disposition", mime.FormatMediaType("inline", map[string]string{"filename": filepath.Base(string(decode(f[2])))}))
			result.Header().Set("Content-Type", "application/octet-stream")
			fault := &os.PathError{Op: "seek", Path: string(decode(f[2])), Err: linuxError(int(integer(f[1])))}
			source := &failingSeeker{Reader: bytes.NewReader([]byte("01234")), fail: 3, failure: fault}
			http.ServeContent(result, request, "no-extension", time.Unix(1, 0), source)
			if f[3] == "HEAD" {
				result.Body.Reset()
			}
			dump(result)
		case "J":
			if len(f) != 5 {
				panic("JSON fault fields")
			}
			path := string(decode(f[3]))
			code := 500
			var failure error
			result := httptest.NewRecorder()
			switch f[1] {
			case "missing":
				code = 404
				failure = fmt.Errorf("path '%s' does not exist", path)
			case "directory":
				code = 400
				failure = fmt.Errorf("path '%s' is a directory", path)
			case "stat":
				errno := int(integer(f[2]))
				if errno == 2 {
					code = 404
					failure = fmt.Errorf("path '%s' does not exist", path)
				} else {
					failure = fmt.Errorf("error checking if path exists '%s': %w", path, &os.PathError{Op: "stat", Path: path, Err: linuxError(errno)})
				}
			case "open":
				result.Header().Set("Vary", "Accept-Encoding")
				failure = fmt.Errorf("error opening file '%s': %w", path, &os.PathError{Op: "open", Path: path, Err: linuxError(int(integer(f[2])))})
			case "encoding":
				code = 406
				failure = errors.New("error parsing Accept-Encoding: no acceptable encoding found, supported: [gzip]")
			case "conditional":
				code = 406
				result.Header().Set("Vary", "Accept-Encoding")
				failure = errors.New("identity encoding not acceptable for Range or conditional request")
			default:
				panic("JSON stage")
			}
			// Fixed envd internal/api/error.go jsonError and api.gen.go Error,
			// runtime f455fd58fa3f289e78ce1de690c35968d8d29322. Messages above
			// reproduce download.go's error branches with controlled PathErrors.
			jsonError(result, code, failure)
			if f[4] == "HEAD" {
				result.Body.Reset()
			}
			dump(result)
		case "P":
			if len(f) != 14 {
				panic("response fields")
			}
			local(f[12], f[13])
			size := integer(f[1])
			if size < 0 || size > 8192 {
				panic("fixture size")
			}
			data := make([]byte, size)
			for i := range data {
				data[i] = byte(i % 251)
			}
			request := &http.Request{Method: f[4], Header: make(http.Header)}
			for i, name := range []string{"If-Match", "If-Unmodified-Since", "If-None-Match", "If-Modified-Since", "If-Range", "Range"} {
				request.Header.Set(name, string(decode(f[6+i])))
			}
			result := httptest.NewRecorder()
			for name, value := range map[string]string{"Content-Type": "application/octet-stream", "Vary": "Accept-Encoding", "Content-Disposition": "inline", "Cache-Control": "private"} {
				result.Header().Set(name, value)
			}
			if f[5] != "" {
				result.Header().Set("ETag", string(decode(f[5])))
			}
			http.ServeContent(result, request, "fixture.bin", time.Unix(integer(f[2]), integer(f[3])), bytes.NewReader(data))
			response := result.Result()
			body := result.Body.Bytes()
			ctype, params, _ := mime.ParseMediaType(response.Header.Get("Content-Type"))
			if ctype == "multipart/byteranges" {
				boundary := params["boundary"]
				if len(boundary) != 60 {
					panic("unexpected Go boundary length")
				}
				normalized := strings.Repeat("a", 60)
				body = bytes.ReplaceAll(body, []byte(boundary), []byte(normalized))
				response.Header.Set("Content-Type", "multipart/byteranges; boundary="+normalized)
			}
			fmt.Printf("%d", response.StatusCode)
			for _, name := range []string{"Last-Modified", "Content-Range", "Accept-Ranges", "Content-Type", "Content-Length", "ETag", "Cache-Control", "Content-Encoding", "X-Content-Type-Options", "Vary", "Content-Disposition"} {
				fmt.Printf("\t%x", response.Header.Get(name))
			}
			fmt.Printf("\t%x\n", body)
			response.Body.Close()
		default:
			panic("unknown operation")
		}
	}
	if err := input.Err(); err != nil {
		panic(err)
	}
}

// Unmodified fixed envd envelope; no injected encoder failure in this oracle.
// Apache-2.0: ../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
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

// Linux syscall text on every test host; native Linux additionally verifies
// the table through syscall.Errno. macOS comparison is not Linux syscall evidence.
func linuxError(n int) error {
	text := "errno " + strconv.Itoa(n)
	if n >= 0 && n < len(linuxErrors) && linuxErrors[n] != "" {
		text = linuxErrors[n]
	}
	if runtime.GOOS == "linux" && runtime.GOARCH == "amd64" && syscall.Errno(n).Error() != text {
		panic("Linux errno mismatch")
	}
	return errors.New(text)
}

// Unchanged Go 1.26.8 syscall/zerrors_linux_amd64.go error table (only variable renamed).
// Copyright 2009 The Go Authors. BSD license: ../../sandbox-guest-runtime/src/multipart/GO-LICENSE.
var linuxErrors = [...]string{
	1:   "operation not permitted",
	2:   "no such file or directory",
	3:   "no such process",
	4:   "interrupted system call",
	5:   "input/output error",
	6:   "no such device or address",
	7:   "argument list too long",
	8:   "exec format error",
	9:   "bad file descriptor",
	10:  "no child processes",
	11:  "resource temporarily unavailable",
	12:  "cannot allocate memory",
	13:  "permission denied",
	14:  "bad address",
	15:  "block device required",
	16:  "device or resource busy",
	17:  "file exists",
	18:  "invalid cross-device link",
	19:  "no such device",
	20:  "not a directory",
	21:  "is a directory",
	22:  "invalid argument",
	23:  "too many open files in system",
	24:  "too many open files",
	25:  "inappropriate ioctl for device",
	26:  "text file busy",
	27:  "file too large",
	28:  "no space left on device",
	29:  "illegal seek",
	30:  "read-only file system",
	31:  "too many links",
	32:  "broken pipe",
	33:  "numerical argument out of domain",
	34:  "numerical result out of range",
	35:  "resource deadlock avoided",
	36:  "file name too long",
	37:  "no locks available",
	38:  "function not implemented",
	39:  "directory not empty",
	40:  "too many levels of symbolic links",
	42:  "no message of desired type",
	43:  "identifier removed",
	44:  "channel number out of range",
	45:  "level 2 not synchronized",
	46:  "level 3 halted",
	47:  "level 3 reset",
	48:  "link number out of range",
	49:  "protocol driver not attached",
	50:  "no CSI structure available",
	51:  "level 2 halted",
	52:  "invalid exchange",
	53:  "invalid request descriptor",
	54:  "exchange full",
	55:  "no anode",
	56:  "invalid request code",
	57:  "invalid slot",
	59:  "bad font file format",
	60:  "device not a stream",
	61:  "no data available",
	62:  "timer expired",
	63:  "out of streams resources",
	64:  "machine is not on the network",
	65:  "package not installed",
	66:  "object is remote",
	67:  "link has been severed",
	68:  "advertise error",
	69:  "srmount error",
	70:  "communication error on send",
	71:  "protocol error",
	72:  "multihop attempted",
	73:  "RFS specific error",
	74:  "bad message",
	75:  "value too large for defined data type",
	76:  "name not unique on network",
	77:  "file descriptor in bad state",
	78:  "remote address changed",
	79:  "can not access a needed shared library",
	80:  "accessing a corrupted shared library",
	81:  ".lib section in a.out corrupted",
	82:  "attempting to link in too many shared libraries",
	83:  "cannot exec a shared library directly",
	84:  "invalid or incomplete multibyte or wide character",
	85:  "interrupted system call should be restarted",
	86:  "streams pipe error",
	87:  "too many users",
	88:  "socket operation on non-socket",
	89:  "destination address required",
	90:  "message too long",
	91:  "protocol wrong type for socket",
	92:  "protocol not available",
	93:  "protocol not supported",
	94:  "socket type not supported",
	95:  "operation not supported",
	96:  "protocol family not supported",
	97:  "address family not supported by protocol",
	98:  "address already in use",
	99:  "cannot assign requested address",
	100: "network is down",
	101: "network is unreachable",
	102: "network dropped connection on reset",
	103: "software caused connection abort",
	104: "connection reset by peer",
	105: "no buffer space available",
	106: "transport endpoint is already connected",
	107: "transport endpoint is not connected",
	108: "cannot send after transport endpoint shutdown",
	109: "too many references: cannot splice",
	110: "connection timed out",
	111: "connection refused",
	112: "host is down",
	113: "no route to host",
	114: "operation already in progress",
	115: "operation now in progress",
	116: "stale file handle",
	117: "structure needs cleaning",
	118: "not a XENIX named type file",
	119: "no XENIX semaphores available",
	120: "is a named type file",
	121: "remote I/O error",
	122: "disk quota exceeded",
	123: "no medium found",
	124: "wrong medium type",
	125: "operation canceled",
	126: "required key not available",
	127: "key has expired",
	128: "key has been revoked",
	129: "key was rejected by service",
	130: "owner died",
	131: "state not recoverable",
	132: "operation not possible due to RF-kill",
}
