// Fixed Go Cmd.Start validation, then the fixed handler.New/Start wrappers.
// Only NUL-bearing inputs are accepted, so validation must prevent a fork.
// This does not run handler.New's cwd Stat, PTY setup or account/cgroup work.
package main

import (
	"os/exec"
	"strings"
	"syscall"
)

func nulStartError(input commandOperationInput) error {
	args := append([]string{"-c", "exit 71", "--", input.Cmd}, input.Args...)
	nul := strings.ContainsRune(input.Cwd, 0)
	for _, arg := range args {
		nul = nul || strings.ContainsRune(arg, 0)
	}
	env := []string{"PATH=/usr/bin:/bin"}
	for key, value := range input.Env {
		item := key + "=" + value
		nul = nul || strings.ContainsRune(item, 0)
		env = append(env, item)
	}
	if !nul {
		panic("NUL fixture must never execute a process")
	}
	cmd := exec.Command("/bin/sh", args...)
	// handler.New always sets a nonnil SysProcAttr. Omitting it causes os to
	// run an extra cwd Stat and return a different PathError on Unix.
	cmd.SysProcAttr = &syscall.SysProcAttr{}
	cmd.Dir = input.Cwd
	cmd.Env = env
	err := cmd.Start()
	if cmd.Process != nil {
		_ = cmd.Process.Kill()
		_ = cmd.Wait()
		panic("NUL validation unexpectedly started a process")
	}
	if err == nil {
		panic("NUL validation unexpectedly succeeded")
	}
	return formatStartError(input, err)
}
