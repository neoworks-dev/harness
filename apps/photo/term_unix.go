//go:build linux || darwin

package main

import (
	"os"
	"syscall"
	"unsafe"
)

func ioctl(f *os.File, req uintptr, arg unsafe.Pointer) error {
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, f.Fd(), req, uintptr(arg))
	if errno != 0 {
		return errno
	}
	return nil
}

// isTerminal reports whether f is a terminal (not just a character device
// such as /dev/null).
func isTerminal(f *os.File) bool {
	var t syscall.Termios
	return ioctl(f, getTermios, unsafe.Pointer(&t)) == nil
}

// terminalSize returns the size of the terminal f is, or zeros.
func terminalSize(f *os.File) (cols, rows int) {
	var ws struct{ Row, Col, X, Y uint16 }
	if ioctl(f, syscall.TIOCGWINSZ, unsafe.Pointer(&ws)) != nil {
		return 0, 0
	}
	return int(ws.Col), int(ws.Row)
}

// makeRaw puts the terminal in raw mode (no echo, no line editing, no signal
// keys: Ctrl-C arrives as a byte) and returns the function that undoes it.
func makeRaw(f *os.File) (func(), error) {
	var old syscall.Termios
	if err := ioctl(f, getTermios, unsafe.Pointer(&old)); err != nil {
		return nil, err
	}
	raw := old
	raw.Iflag &^= syscall.IGNBRK | syscall.BRKINT | syscall.PARMRK | syscall.ISTRIP | syscall.INLCR | syscall.IGNCR | syscall.ICRNL | syscall.IXON
	raw.Lflag &^= syscall.ECHO | syscall.ECHONL | syscall.ICANON | syscall.ISIG | syscall.IEXTEN
	raw.Cflag &^= syscall.CSIZE | syscall.PARENB
	raw.Cflag |= syscall.CS8
	raw.Cc[syscall.VMIN], raw.Cc[syscall.VTIME] = 1, 0
	if err := ioctl(f, setTermios, unsafe.Pointer(&raw)); err != nil {
		return nil, err
	}
	return func() { ioctl(f, setTermios, unsafe.Pointer(&old)) }, nil
}
