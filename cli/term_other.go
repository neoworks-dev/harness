//go:build !unix

package main

import (
	"errors"
	"os"
	osexec "os/exec"
)

// terminalSize is unknown here; callers fall back to $COLUMNS or 80.
func terminalSize(*os.File) (cols, rows int) { return 0, 0 }

func detach(*osexec.Cmd) {}

func execReplace(string, []string, []string) error {
	return errors.New("not supported on this platform")
}

func killGroup(int) {}

func terminate(int) error { return errors.New("not supported on this platform") }
