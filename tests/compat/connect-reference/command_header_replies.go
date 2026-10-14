// Inspect original error bytes after decompression, without normalizing JSON
// escaping, stream flags or HTTP metadata. Gzip bitstreams need not be identical.
package main

import (
	"encoding/binary"
	"encoding/hex"
	"net/http"
	"strconv"
	"strings"
)

func commandHeaderReply(res *http.Response, body []byte) string {
	result := []string{strconv.Itoa(res.StatusCode)}
	for _, name := range []string{"Content-Type", "Content-Encoding", "Connect-Content-Encoding", "Accept-Encoding", "Connect-Accept-Encoding", "Content-Length"} {
		result = append(result, hex.EncodeToString([]byte(res.Header.Get(name))))
	}
	flag := ""
	if strings.HasPrefix(res.Header.Get("Content-Type"), "application/connect+") {
		if len(body) < 5 || int(binary.BigEndian.Uint32(body[1:5])) != len(body)-5 {
			panic("header rejection must contain exactly one complete frame")
		}
		flag = strconv.Itoa(int(body[0]))
		compressed := body[0]&1 != 0
		body = body[5:]
		if compressed {
			body = commandHeaderExpand(body)
		}
	}
	result = append(result, flag, hex.EncodeToString(body))
	return strings.Join(result, "\t")
}
