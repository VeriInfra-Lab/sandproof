// Fixed Go 1.26.8 upload header oracle; original mime.ParseMediaType and
// fixed envd extraction/validation (f455fd58fa3f289e78ce1de690c35968d8d29322).
// Apache-2.0: ../../sandbox-guest-runtime/src/file_encoding/E2B-LICENSE.
package main

import (
	"bufio"
	"bytes"
	"compress/gzip"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"mime/multipart"
	"net/http"
	"net/http/httptest"
	"net/textproto"
	"os"
	"os/user"
	"path/filepath"
	"runtime"
	"slices"
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

// pkg/xattr v0.4.12 xattr.go (e34e274d513734a021b7da0ad9f497859fadf639).
// Original Error type/methods, type name changed only.
// BSD license: ../../sandbox-guest-service/src/upload_response/XATTR-LICENSE.
// Error records an error and the operation, file path and attribute that caused it.
type oracleXattrError struct {
	Op   string
	Path string
	Name string
	Err  error
}

func (e *oracleXattrError) Unwrap() error { return e.Err }

func (e *oracleXattrError) Error() (errstr string) {
	if e.Op != "" {
		errstr += e.Op
	}
	if e.Path != "" {
		if errstr != "" {
			errstr += " "
		}
		errstr += e.Path
	}
	if e.Name != "" {
		if errstr != "" {
			errstr += " "
		}
		errstr += e.Name
	}
	if e.Err != nil {
		if errstr != "" {
			errstr += ": "
		}
		errstr += e.Err.Error()
	}
	return
}

// Fixed permissions/path.go directory preparation, I/O names changed only.
func getSubpaths(path string) (subpaths []string) {
	for {
		subpaths = append(subpaths, path)

		path = filepath.Dir(path)
		if path == "/" {
			break
		}
	}

	slices.Reverse(subpaths)

	return subpaths
}

func oracleEnsureDirs(path string, uid, gid int) error {
	subpaths := getSubpaths(path)
	for _, subpath := range subpaths {
		info, err := oracleParentStat(subpath)
		if err != nil && !os.IsNotExist(err) {
			return fmt.Errorf("failed to stat directory: %w", err)
		}

		if err != nil && os.IsNotExist(err) {
			err = oracleMkdir(subpath, 0o755)
			if err != nil && !os.IsExist(err) {
				return fmt.Errorf("failed to create directory: %w", err)
			}

			// Chown even if another process created the directory (os.IsExist),
			// since it may not have called Chown yet. Chown is idempotent.
			err = oracleParentChown(subpath, uid, gid)
			if err != nil {
				return fmt.Errorf("failed to chown directory: %w", err)
			}

			continue
		}

		if !info.IsDir() {
			return fmt.Errorf("path is a file: %s", subpath)
		}
	}

	return nil
}

const MetadataXattrPrefix = "user.e2b."

// Original generated fields and api/error.go jsonError.
type Error struct {
	Code    int    `json:"code"`
	Message string `json:"message"`
}

func jsonError(w http.ResponseWriter, code int, err error) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.Header().Set("X-Content-Type-Options", "nosniff")

	w.WriteHeader(code)
	encodeErr := json.NewEncoder(w).Encode(Error{
		Code:    code,
		Message: err.Error(),
	})
	if encodeErr != nil {
		http.Error(w, errors.Join(encodeErr, err).Error(), http.StatusInternalServerError)
	}
}

// Controlled observations only; these shims perform no filesystem mutation.
// Translate Linux errno identity for a reference built on macOS as well.
var oracleStage string
var oracleFailure error

type oracleErrno int

func (e oracleErrno) Error() string { return linuxError(int(e)).Error() }
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

var oraclePath = "/file"
var oracleAttribute string

func oracleParentStat(path string) (os.FileInfo, error) {
	if path == oraclePath {
		switch oracleStage {
		case "ParentStat":
			return nil, &os.PathError{Op: "stat", Path: path, Err: oracleFailure}
		case "ParentCreate", "ParentChown":
			return nil, os.ErrNotExist
		case "ParentNotDirectory":
			return oracleInfo{directory: false}, nil
		}
	}
	return oracleInfo{directory: true}, nil
}
func oracleMkdir(path string, mode os.FileMode) error {
	if oracleStage == "ParentCreate" {
		return &os.PathError{Op: "mkdir", Path: path, Err: oracleFailure}
	}
	return nil
}
func oracleParentChown(path string, uid, gid int) error {
	if oracleStage == "ParentChown" {
		return &os.PathError{Op: "chown", Path: path, Err: oracleFailure}
	}
	return nil
}
func oracleFilePath() string {
	if strings.HasPrefix(oracleStage, "Parent") {
		return oraclePath + "/file"
	}
	return oraclePath
}

type oracleInfo struct {
	os.FileInfo
	directory bool
}

func (i oracleInfo) IsDir() bool { return i.directory }
func oracleStat(path string) (os.FileInfo, error) {
	if oracleStage == "Stat" {
		return nil, &os.PathError{Op: "stat", Path: path, Err: oracleFailure}
	}
	if oracleStage == "FileChown" {
		return nil, os.ErrNotExist
	}
	return oracleInfo{directory: oracleStage == "Directory"}, nil
}
func oracleChown(path string, uid, gid int) error {
	if oracleStage == "Chown" {
		return &os.PathError{Op: "chown", Path: path, Err: oracleFailure}
	}
	return nil
}

type oracleFile struct{}

var oracleCopied bytes.Buffer

func oracleOpenFile(path string, flag int, mode os.FileMode) (*oracleFile, error) {
	if oracleStage == "Open" {
		return nil, &os.PathError{Op: "open", Path: path, Err: oracleFailure}
	}
	return &oracleFile{}, nil
}
func (*oracleFile) Close() error { return nil }
func (*oracleFile) Chown(int, int) error {
	if oracleStage == "FileChown" {
		return &os.PathError{Op: "chown", Path: oraclePath, Err: oracleFailure}
	}
	return nil
}
func (*oracleFile) ReadFrom(reader io.Reader) (int64, error) {
	if oracleStage == "RawGzip" {
		return oracleCopied.ReadFrom(reader)
	}
	if oracleStage == "Write" {
		return 0, &os.PathError{Op: "write", Path: oraclePath, Err: oracleFailure}
	}
	if oracleStage == "WriteZero" {
		return 0, io.ErrShortWrite
	}
	return 4, nil
}
func oracleWriteMetadata(*oracleFile, map[string]string) error {
	if strings.HasPrefix(oracleStage, "Metadata") {
		op := map[string]string{"MetadataList": "xattr.list", "MetadataRemove": "xattr.FRemove", "MetadataSet": "xattr.FSet"}[oracleStage]
		return &oracleXattrError{Op: op, Path: oraclePath, Name: oracleAttribute, Err: oracleFailure}
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
	dumpUpload(response)
}

// Original resolvePath from fixed upload.go (blob 765e8751ff6fd0901a603b48126ca8292a2e2fe0).
// Only utils/permissions call namespaces change. Expansion returns an injected,
// already resolved observation: no account/path expansion or filesystem claim.
type UploadSuccess []EntryInfo
type PostFilesParams struct{ Path *string }

func oracleExpandAndResolve(path string, _ *user.User, _ *string) (string, error) {
	return path, nil
}
func resolvePath(part *multipart.Part, paths *UploadSuccess, u *user.User, defaultPath *string, params PostFilesParams) (string, error) {
	var pathToResolve string

	if params.Path != nil {
		pathToResolve = *params.Path
	} else {
		var err error
		customPart := NewCustomPart(part)
		pathToResolve, err = customPart.FileNameWithPath()
		if err != nil {
			return "", fmt.Errorf("error getting multipart custom part file name: %w", err)
		}
	}

	filePath, err := oracleExpandAndResolve(pathToResolve, u, defaultPath)
	if err != nil {
		return "", fmt.Errorf("error resolving path: %w", err)
	}

	for _, entry := range *paths {
		if entry.Path == filePath {
			var alreadyUploaded []string
			for _, uploadedFile := range *paths {
				if uploadedFile.Path != filePath {
					alreadyUploaded = append(alreadyUploaded, uploadedFile.Path)
				}
			}

			errMsg := fmt.Errorf("you cannot upload multiple files to the same path '%s' in one upload request, only the first specified file was uploaded", filePath)

			if len(alreadyUploaded) > 1 {
				errMsg = fmt.Errorf("%w, also the following files were uploaded: %v", errMsg, strings.Join(alreadyUploaded, ", "))
			}

			return "", errMsg
		}
	}

	return filePath, nil
}

// Original internal/utils/multipart.go (blob a03bfd94669b1be2fb0ac814be0b6fbd22f150bb).
type CustomPart struct {
	*multipart.Part
}

// FileNameWithPath returns the filename parameter of the Part's Content-Disposition header.
// This method borrows from the original FileName method implementation but returns the full
// filename without using `filepath.Base`.
func (p *CustomPart) FileNameWithPath() (string, error) {
	dispositionParams, err := p.parseContentDisposition()
	if err != nil {
		return "", err
	}
	filename, ok := dispositionParams["filename"]
	if !ok {
		return "", errors.New("filename not found in Content-Disposition header")
	}

	return filename, nil
}

func (p *CustomPart) parseContentDisposition() (map[string]string, error) {
	v := p.Header.Get("Content-Disposition")
	_, dispositionParams, err := mime.ParseMediaType(v)
	if err != nil {
		return nil, err
	}

	return dispositionParams, nil
}

// NewCustomPart creates a new CustomPart from a multipart.Part
func NewCustomPart(part *multipart.Part) *CustomPart {
	return &CustomPart{Part: part}
}

func multipartRejection(fields []string) error {
	paths := UploadSuccess{}
	var params PostFilesParams
	part := &multipart.Part{Header: make(textproto.MIMEHeader)}
	switch fields[0] {
	case "D":
		path := string(unhex(fields[1]))
		params.Path = &path
		for _, value := range fields[2:] {
			paths = append(paths, EntryInfo{Path: string(unhex(value))})
		}
	case "N":
		part.Header.Set("Content-Disposition", string(unhex(fields[1])))
		// Original handlePart gate precedes resolvePath and custom filename.
		if part.FormName() != "file" {
			panic("expected file part")
		}
	default:
		panic("multipart operation")
	}
	_, err := resolvePath(part, &paths, nil, nil, params)
	if err == nil {
		panic("expected multipart rejection")
	}
	return err
}

// Exercise fixed Go request/reader preparation without reading a real body.
// Reaching the supplied reader means setup succeeded; its synthetic failure is
// never published as a server error. Empty boundary fails before this point.
type setupProbe struct{ reads int }

func (p *setupProbe) Read([]byte) (int, error) {
	p.reads++
	return 0, errors.New("reference reached body input")
}
func multipartSetup(fields []string) error {
	probe := &setupProbe{}
	r := &http.Request{Header: make(http.Header), Body: io.NopCloser(probe)}
	for _, field := range fields {
		r.Header.Add("Content-Type", string(unhex(field)))
	}
	f, err := r.MultipartReader()
	if err != nil {
		if probe.reads != 0 {
			panic("setup rejection read body")
		}
		// Original handleMultipartUpload wrapper and status 500.
		return fmt.Errorf("error parsing multipart form: %w", err)
	}
	_, err = f.NextPart()
	if probe.reads != 0 {
		return nil
	}
	if err == nil {
		panic("part without input")
	}
	// Original handleMultipartUpload NextPart failure wrapper and status 500.
	return fmt.Errorf("error reading form: %w", err)
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
		case "D":
			if server == nil {
				server = referenceServer()
			}
			rawGzipBody(f[1], server)
		case "P", "G", "B", "T":
			if server == nil {
				server = referenceServer()
			}
			var err error
			status := 400
			if f[0] == "P" {
				err = headerRejection(f[1:])
			} else if f[0] == "B" {
				err = multipartRejection(f[1:])
			} else if f[0] == "T" {
				err = multipartSetup(f[1:])
				if err == nil {
					fmt.Println("READY")
					continue
				}
				status = 500
			} else {
				r := &http.Request{Header: make(http.Header), Body: io.NopCloser(bytes.NewReader(unhex(f[1])))}
				r.Header.Set("Content-Encoding", "gzip")
				body, failure := getDecompressedBody(r)
				if failure == nil {
					body.Close()
					fmt.Println("READY")
					continue
				}
				// Original PostFiles wrapping, before user.Lookup or file effects.
				err = fmt.Errorf("error decompressing request body: %w", failure)
			}
			request, failure := http.NewRequest("POST", server.URL, strings.NewReader(err.Error()))
			if failure != nil {
				panic(failure)
			}
			request.Header.Set("X-Reference-Code", strconv.Itoa(status))
			client := server.Client()
			client.Timeout = 5 * time.Second
			response, failure := client.Do(request)
			if failure != nil {
				panic(failure)
			}
			dumpUpload(response)
		case "F", "J":
			oracleStage = f[1]
			oraclePath, oracleAttribute = "/file", ""
			length := int64(4)
			if f[0] == "J" {
				oraclePath, oracleAttribute = string(unhex(f[3])), string(unhex(f[4]))
				var err error
				length, err = strconv.ParseInt(f[5], 10, 64)
				if err != nil {
					panic(err)
				}
			}
			n, err := strconv.Atoi(f[2])
			if err != nil {
				panic(err)
			}
			oracleFailure = oracleErrno(n)
			status, err := processFile(&http.Request{ContentLength: length}, oracleFilePath(), strings.NewReader("body"), 0, 0, nil, oracleLogger{})
			if err == nil {
				panic("expected injected failure")
			}
			if f[0] == "F" {
				fmt.Println(status)
			} else {
				if server == nil {
					server = referenceServer()
				}
				request, failure := http.NewRequest("POST", server.URL, strings.NewReader(err.Error()))
				if failure != nil {
					panic(failure)
				}
				request.Header.Set("X-Reference-Code", strconv.Itoa(status))
				client := server.Client()
				client.Timeout = 5 * time.Second
				response, failure := client.Do(request)
				if failure != nil {
					panic(failure)
				}
				dumpUpload(response)
			}
		case "R":
			if server == nil {
				server = referenceServer()
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

// Original getDecompressedBody and processFile consume a real fixed gzip
// reader. Only filesystem calls are controlled; ReadFrom retains the exact
// decoded prefix in memory. No Linux disk/full/metadata execution is claimed.
func rawGzipBody(encoded string, server *httptest.Server) {
	data := unhex(encoded)
	r := &http.Request{Header: make(http.Header), Body: io.NopCloser(bytes.NewReader(data)), ContentLength: int64(len(data))}
	r.Header.Set("Content-Encoding", "gzip")
	body, err := getDecompressedBody(r)
	if err != nil {
		panic("expected completed initial gzip header")
	}
	defer body.Close()
	oracleStage, oraclePath = "RawGzip", "/file"
	oracleCopied.Reset()
	status, err := processFile(r, oraclePath, body, 0, 0, nil, oracleLogger{})
	fmt.Printf("%x\t", oracleCopied.Bytes())
	if err == nil {
		fmt.Println("END")
		return
	}
	request, failure := http.NewRequest("POST", server.URL, strings.NewReader(err.Error()))
	if failure != nil {
		panic(failure)
	}
	request.Header.Set("X-Reference-Code", strconv.Itoa(status))
	client := server.Client()
	client.Timeout = 5 * time.Second
	response, failure := client.Do(request)
	if failure != nil {
		panic(failure)
	}
	dumpUpload(response)
}

func referenceServer() *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		data, err := io.ReadAll(r.Body)
		if err != nil {
			panic(err)
		}
		if code := r.Header.Get("X-Reference-Code"); code != "" {
			n, err := strconv.Atoi(code)
			if err != nil {
				panic(err)
			}
			jsonError(w, n, errors.New(string(data)))
			return
		}
		// Original PostFiles success writes: no explicit Content-Type.
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write(data)
	}))
}

func dumpUpload(response *http.Response) {
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil {
		panic(err)
	}
	fmt.Printf("%d\t%x\t%x\t%x\t%x\n", response.StatusCode, response.Header.Get("Content-Type"), response.Header.Get("Content-Length"), response.Header.Get("X-Content-Type-Options"), body)
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

// Original encoding.go methods, blob df8d54f70f1b685a7b77032bbb3d6eb762e4fc13.
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

// getDecompressedBody returns a reader that decompresses the request body based on
// Content-Encoding header. Returns the original body if no encoding is specified.
// Returns an error if an unsupported encoding is specified.
// The caller is responsible for closing both the returned ReadCloser and the
// original request body (r.Body) separately.
func getDecompressedBody(r *http.Request) (io.ReadCloser, error) {
	encoding, err := parseContentEncoding(r)
	if err != nil {
		return nil, err
	}

	if encoding == EncodingIdentity {
		return r.Body, nil
	}

	switch encoding {
	case EncodingGzip:
		gzReader, err := gzip.NewReader(r.Body)
		if err != nil {
			return nil, fmt.Errorf("failed to create gzip reader: %w", err)
		}

		return gzReader, nil
	default:
		// This shouldn't happen if isSupportedEncoding is correct
		return nil, fmt.Errorf("encoding %s is supported but not implemented", encoding)
	}
}

// Only the rejection expressions from PostFiles / handleRawUpload are used
// here. This is not a replacement API, account lookup, body decoder test or
// complete request ordering oracle.
func headerRejection(fields []string) error {
	r := &http.Request{Header: make(http.Header), Body: io.NopCloser(strings.NewReader(""))}
	for _, field := range fields[1:] {
		kv := strings.SplitN(field, ":", 2)
		r.Header.Add(string(unhex(kv[0])), string(unhex(kv[1])))
	}
	switch fields[0] {
	case "E":
		body, err := getDecompressedBody(r)
		if err != nil {
			return fmt.Errorf("error decompressing request body: %w", err)
		}
		body.Close()
	case "C":
		contentType := r.Header.Get("Content-Type")
		mediaType, _, _ := mime.ParseMediaType(contentType)
		if mediaType != "application/octet-stream" && !strings.HasPrefix(mediaType, "multipart/") {
			return fmt.Errorf("unsupported content type: %s, expected multipart/form-data or application/octet-stream", contentType)
		}
	case "R":
		return errors.New("path query parameter is required for raw body upload")
	case "M":
		metadata := extractMetadataHeaders(r.Header)
		if err := ValidateMetadata(metadata); err != nil {
			return fmt.Errorf("invalid metadata: %w", err)
		}
	}
	panic("expected rejected input")
}
