// Command photo is a terminal photo editor, a stand-in app for the suite. It
// registers as "photo-editor" with the daemon, serves its image tools to the
// agent over MCP and shows the current image with the shared chat.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"os/signal"
	"syscall"

	sdk "neoworks.dev/suite/sdk-go"
)

func main() {
	os.Exit(realMain())
}

func realMain() int {
	socket := flag.String("socket", "", "daemon socket (default: $SUITE_SOCKET, else the per-user runtime dir)")
	headless := flag.Bool("headless", false, "no terminal UI: serve the tools only (the default when stdout is not a terminal)")
	flag.Usage = func() {
		fmt.Fprintln(os.Stderr, "usage: photo [flags] [image file]")
		flag.PrintDefaults()
	}
	flag.Parse()
	if flag.NArg() > 1 {
		flag.Usage()
		return 2
	}

	cfg := config{
		Socket:   sdk.SocketPath(*socket, os.Getenv, os.Getuid()),
		Headless: *headless,
		Term:     stdTerm{os.Stdin, os.Stdout},
		Stderr:   os.Stderr,
		Getenv:   os.Getenv,
	}
	if flag.NArg() == 1 {
		cfg.Open = flag.Arg(0)
	}
	if exe, err := os.Executable(); err == nil {
		cfg.Launch = []string{exe}
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	if err := run(ctx, cfg); err != nil {
		fmt.Fprintf(os.Stderr, "photo: %v\n", err)
		return 1
	}
	return 0
}
