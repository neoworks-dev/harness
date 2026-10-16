package main

import "os"

// terminal is the process's controlling terminal as far as the UI needs it.
type terminal interface {
	interactive() bool // stdin and stdout are both terminals
	size() (cols, rows int)
	// raw switches stdin to raw mode and returns the function that restores it.
	raw() (restore func(), err error)
	in() *os.File
	out() *os.File
}

// stdTerm is the real terminal.
type stdTerm struct{ stdin, stdout *os.File }

func (t stdTerm) in() *os.File  { return t.stdin }
func (t stdTerm) out() *os.File { return t.stdout }

func (t stdTerm) interactive() bool { return isTerminal(t.stdin) && isTerminal(t.stdout) }

func (t stdTerm) size() (int, int) { return terminalSize(t.stdout) }

func (t stdTerm) raw() (func(), error) { return makeRaw(t.stdin) }
