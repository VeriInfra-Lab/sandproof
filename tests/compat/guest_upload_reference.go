// Fixed Go 1.26.8 upload header oracle; original mime.ParseMediaType and
// fixed envd extraction/validation (f455fd58fa3f289e78ce1de690c35968d8d29322).
// Apache-2.0: ../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
	"syscall"
	"time"
)

// Original processFile from packages/envd/internal/api/upload.go, blob
// 765e8751ff6fd0901a603b48126ca8292a2e2fe0 at the fixed runtime commit above.
// Only logger type and I/O namespace calls are substituted; status/error
// branches remain original. Fault injection checks status classification, not
// actual disk-full behavior, kernel I/O, or a complete envd HTTP response.
var ErrNoDiskSpace = errors.New("not enough disk space available")

func processFile(r *http.Request, path string, part io.Reader, uid, gid int, metadata map[string]string, logger oracleLogger) (int, error) {
	logger.Debug().
		Str("path", path).
		Msg("File processing")

	err := oracleEnsureDirs(filepath.Dir(path), uid, gid)
	if err != nil {
		err := fmt.Errorf("error ensuring directories: %w", err)

		return http.StatusInternalServerError, err
	}

	canBePreChowned := false
	stat, err := oracleStat(path)
	if err != nil && !os.IsNotExist(err) {
		errMsg := fmt.Errorf("error getting file info: %w", err)

		return http.StatusInternalServerError, errMsg
	} else if err == nil {
		if stat.IsDir() {
			err := fmt.Errorf("path is a directory: %s", path)

			return http.StatusBadRequest, err
		}
		canBePreChowned = true
	}

	hasBeenChowned := false
	if canBePreChowned {
		err = oracleChown(path, uid, gid)
		if err != nil {
			if !os.IsNotExist(err) {
				err = fmt.Errorf("error changing file ownership: %w", err)

				return http.StatusInternalServerError, err
			}
		} else {
			hasBeenChowned = true
		}
	}

	file, err := oracleOpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, 0o666)
	if err != nil {
		if errors.Is(err, syscall.ENOSPC) {
			err = fmt.Errorf("not enough inodes available: %w", err)

			return http.StatusInsufficientStorage, err
		}

		err := fmt.Errorf("error opening file: %w", err)

		return http.StatusInternalServerError, err
	}

	defer file.Close()

	if !hasBeenChowned {
		err = file.Chown(uid, gid)
		if err != nil {
			err := fmt.Errorf("error changing file ownership: %w", err)

			return http.StatusInternalServerError, err
		}
	}

	_, err = file.ReadFrom(part)
	if err != nil {
		if errors.Is(err, syscall.ENOSPC) {
			err = ErrNoDiskSpace
			if r.ContentLength > 0 {
				err = fmt.Errorf("attempted to write %d bytes: %w", r.ContentLength, err)
			}

			return http.StatusInsufficientStorage, err
		}

		err = fmt.Errorf("error writing file: %w", err)

		return http.StatusInternalServerError, err
	}

	// Always (re)write metadata, even with an empty/nil map, so that
	// overwriting a file replaces its full metadata set: keys absent from
	// this request are cleared (O_TRUNC truncates the body but preserves
	// xattrs from a prior upload).
	if err := oracleWriteMetadata(file, metadata); err != nil {
		switch {
		case oracleIsXattrUnsupported(err):
			// Filesystem doesn't support xattrs. ext4 (the sandbox rootfs)
			// always supports them; this branch only triggers for virtual
			// filesystems such as /sys and /proc that the upload API also
			// supports. The file body was already persisted, so we log and
			// continue; the response EntryInfo reads xattrs back from disk
			// so it won't falsely claim metadata was persisted.
			logger.Warn().Str("path", path).Err(err).Msg("filesystem does not support xattrs; metadata not persisted")
		case errors.Is(err, syscall.ENOSPC) || errors.Is(err, syscall.EDQUOT):
			return http.StatusInsufficientStorage, fmt.Errorf("not enough space for file metadata: %w", err)
		default:
			return http.StatusInternalServerError, fmt.Errorf("error writing file metadata: %w", err)
		}
	}

	return http.StatusNoContent, nil
}

const MetadataXattrPrefix = "user.e2b."

// Controlled observations only; these shims perform no filesystem mutation.
// Translate Linux errno identity for a reference built on macOS as well.
var oracleStage string
var oracleFailure error

type oracleErrno int

func (e oracleErrno) Error() string { return fmt.Sprintf("injected Linux errno %d", e) }
func (e oracleErrno) Is(target error) bool {
	return (e == 28 && target == syscall.ENOSPC) ||
		(e == 122 && target == syscall.EDQUOT)
}

type oracleLogger struct{}

func (l oracleLogger) Debug() oracleLogger             { return l }
func (l oracleLogger) Warn() oracleLogger              { return l }
func (l oracleLogger) Str(string, string) oracleLogger { return l }
func (l oracleLogger) Err(error) oracleLogger          { return l }
func (oracleLogger) Msg(string)                        {}
func oracleEnsureDirs(string, int, int) error {
	if strings.HasPrefix(oracleStage, "Parent") {
		return oracleFailure
	}
	return nil
}

type oracleInfo struct{ os.FileInfo }

func (oracleInfo) IsDir() bool { return oracleStage == "Directory" }
func oracleStat(string) (os.FileInfo, error) {
	if oracleStage == "Stat" {
		return nil, oracleFailure
	}
	if oracleStage == "FileChown" {
		return nil, os.ErrNotExist
	}
	return oracleInfo{}, nil
}
func oracleChown(string, int, int) error {
	if oracleStage == "Chown" {
		return oracleFailure
	}
	return nil
}

type oracleFile struct{}

func oracleOpenFile(string, int, os.FileMode) (*oracleFile, error) {
	if oracleStage == "Open" {
		return nil, oracleFailure
	}
	return &oracleFile{}, nil
}
func (*oracleFile) Close() error { return nil }
func (*oracleFile) Chown(int, int) error {
	if oracleStage == "FileChown" {
		return oracleFailure
	}
	return nil
}
func (*oracleFile) ReadFrom(io.Reader) (int64, error) {
	if oracleStage == "Write" {
		return 0, oracleFailure
	}
	if oracleStage == "WriteZero" {
		return 0, io.ErrShortWrite
	}
	return 4, nil
}
func oracleWriteMetadata(*oracleFile, map[string]string) error {
	if strings.HasPrefix(oracleStage, "Metadata") {
		return oracleFailure
	}
	return nil
}
func oracleIsXattrUnsupported(err error) bool {
	return errors.Is(err, syscall.ENOTSUP) || errors.Is(err, syscall.EOPNOTSUPP)
}

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

// Original generated wire fields and pinned upload success construction.
type EntryInfo struct {
	Metadata *map[string]string `json:"metadata,omitempty"`
	Name     string             `json:"name"`
	Path     string             `json:"path"`
	Type     EntryInfoType      `json:"type"`
}
type EntryInfoType string

func uploadSuccess(fields []string, server *httptest.Server) {
	entries := []EntryInfo{}
	for _, field := range fields {
		values := strings.Split(field, "|")
		path := string(unhex(values[0]))
		metadata := make(map[string]string)
		for _, pair := range values[1:] {
			kv := strings.SplitN(pair, ":", 2)
			metadata[string(unhex(kv[0]))] = string(unhex(kv[1]))
		}
		entry := EntryInfo{Path: path, Name: filepath.Base(path), Type: "file"}
		if len(metadata) > 0 {
			entry.Metadata = &metadata
		}
		entries = append(entries, entry)
	}
	data, err := json.Marshal(entries)
	if err != nil {
		panic(err)
	}
	client := server.Client()
	client.Timeout = 5 * time.Second
	response, err := client.Post(server.URL, "application/octet-stream", bytes.NewReader(data))
	if err != nil {
		panic(err)
	}
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		panic(err)
	}
	fmt.Printf("%d\t%x\t%x\t%x\t%x\n", response.StatusCode, response.Header.Get("Content-Type"), response.Header.Get("Content-Length"), response.Header.Get("X-Content-Type-Options"), body)
}
func main() {
	if runtime.Version() != "go1.26.8" {
		panic("requires go1.26.8")
	}
	var server *httptest.Server
	defer func() {
		if server != nil {
			server.Close()
		}
	}()
	input := bufio.NewScanner(os.Stdin)
	input.Buffer(make([]byte, 4096), 1<<20)
	for input.Scan() {
		f := strings.Split(input.Text(), "\t")
		switch f[0] {
		case "F":
			oracleStage = f[1]
			n, err := strconv.Atoi(f[2])
			if err != nil {
				panic(err)
			}
			oracleFailure = oracleErrno(n)
			status, err := processFile(&http.Request{ContentLength: 4}, "/file", strings.NewReader("body"), 0, 0, nil, oracleLogger{})
			if err == nil {
				panic("expected injected failure")
			}
			fmt.Println(status)
		case "R":
			if server == nil {
				server = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					data, err := io.ReadAll(r.Body)
					if err != nil {
						panic(err)
					}
					// Original PostFiles success writes: no explicit Content-Type.
					w.WriteHeader(http.StatusOK)
					_, _ = w.Write(data)
				}))
			}
			uploadSuccess(f[1:], server)
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
