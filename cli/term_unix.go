//go:build unix

package main

import (
	"os"
	"syscall"
	"unsafe"
)

// terminalSize returns the size of the terminal f is, or zeros.
func terminalSize(f *os.File) (cols, rows int) {
	var ws struct{ Row, Col, X, Y uint16 }
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, f.Fd(), syscall.TIOCGWINSZ, uintptr(unsafe.Pointer(&ws)))
	if errno != 0 {
		return 0, 0
	}
	return int(ws.Col), int(ws.Row)
}
