//go:build !linux && !darwin

package main

import (
	"errors"
	"os"
)

// Without termios the UI cannot run: the app stays headless.
func isTerminal(*os.File) bool { return false }

func terminalSize(*os.File) (cols, rows int) { return 0, 0 }

func makeRaw(*os.File) (func(), error) {
	return nil, errors.New("raw terminal mode is not supported here")
}
