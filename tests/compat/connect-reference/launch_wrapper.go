// Controlled helper availability and priority from runtime f455fd58fa3f289e78ce1de690c35968d8d29322
// process/handler/handler.go New/ioniceNicePrefix, plus original filepath.Join.
// This checks exact construction only, not helper discovery or command execution.
package main

import (
	"encoding/json"
	"fmt"
	"path/filepath"
)

func launchWrapper(data []byte) []byte {
	var input struct {
		Ionice, Nice    *string
		CurrentNice     int
		Directory, Name string
	}
	if err := json.Unmarshal(data, &input); err != nil {
		panic(err)
	}
	prefix := ""
	if input.Ionice != nil {
		prefix += fmt.Sprintf("%s -c %d -n %d ", *input.Ionice, 2, 4)
	}
	if input.Nice != nil {
		prefix += fmt.Sprintf("%s -n %d ", *input.Nice, 0-input.CurrentNice)
	}
	output := struct{ Script, Candidate string }{
		fmt.Sprintf(`echo %d > /proc/$$/oom_score_adj && exec %s"${@}"`, 100, prefix),
		filepath.Join(input.Directory, input.Name),
	}
	encoded, err := json.Marshal(output)
	if err != nil {
		panic(err)
	}
	return encoded
}
