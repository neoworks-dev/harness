//go:build unix

package main

import (
	"os"
	osexec "os/exec"
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

// detach makes cmd a session leader so it outlives this process and the terminal.
func detach(cmd *osexec.Cmd) { cmd.SysProcAttr = &syscall.SysProcAttr{Setsid: true} }

// execReplace replaces this process with path.
func execReplace(path string, argv, env []string) error { return syscall.Exec(path, argv, env) }

// killGroup kills the process group led by pid.
func killGroup(pid int) { syscall.Kill(-pid, syscall.SIGKILL) }

// terminate asks the process to shut down.
func terminate(pid int) error { return syscall.Kill(pid, syscall.SIGTERM) }
