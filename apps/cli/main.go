// Command suite is the CLI for the suite daemon.
package main

import (
	"context"
	"os"
	"os/signal"
	"time"
)

func main() {
	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt)
	os.Exit(run(context.Background(), os.Args[1:], cliEnv{
		Stdin:     os.Stdin,
		Stdout:    os.Stdout,
		Stderr:    os.Stderr,
		Getenv:    os.Getenv,
		UID:       os.Getuid(),
		Interrupt: interrupt,
		Now:       time.Now,
		StdoutTTY: isTerminal(os.Stdout),
		Size:      func() (int, int) { return terminalSize(os.Stdout) },
	}))
}
