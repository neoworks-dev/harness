package main

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	osexec "os/exec"
	"path/filepath"
	"strings"
	"time"

	sdk "neoworks.dev/neo/sdk-go"
)

const (
	// upTimeout bounds how long `neo up` waits for a started daemon to answer.
	upTimeout = 15 * time.Second
	// downTimeout bounds how long `neo down` waits for the socket to go quiet.
	downTimeout = 10 * time.Second
	// pollEvery is the interval between socket probes.
	pollEvery = 100 * time.Millisecond
	// logTailLines is how much of the daemon log to show when it fails to start.
	logTailLines = 20
)

// daemonEntry is the daemon's entry point relative to the repo root.
var daemonEntry = filepath.Join("daemon", "src", "main.ts")

// probe dials the daemon once. It returns the daemon's pid if it answers.
func probe(ctx context.Context, socket string) (pid int, err error) {
	ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	c, err := sdk.Dial(ctx, sdk.Options{Socket: socket, Client: "cli"})
	if err != nil {
		return 0, err
	}
	defer c.Close()
	return int(c.Welcome().PID), nil
}

// dataDir resolves the daemon's data directory like daemon/src/paths.ts.
func dataDir(getenv func(string) string) string {
	if d := getenv("NEO_DATA_DIR"); d != "" {
		return d
	}
	if d := getenv("XDG_DATA_HOME"); d != "" {
		return filepath.Join(d, "neo")
	}
	home := getenv("HOME")
	if home == "" {
		home, _ = os.UserHomeDir()
	}
	return filepath.Join(home, ".local", "share", "neo")
}

// findRoot walks up from the real path of exe to the directory that holds
// daemon/src/main.ts (bin/neo -> the repo root).
func findRoot(exe string) (string, error) {
	real, err := filepath.EvalSymlinks(exe)
	if err != nil {
		real = exe
	}
	for dir := filepath.Dir(real); ; dir = filepath.Dir(dir) {
		if st, err := os.Stat(filepath.Join(dir, daemonEntry)); err == nil && !st.IsDir() {
			return dir, nil
		}
		if dir == filepath.Dir(dir) {
			break
		}
	}
	return "", fmt.Errorf("cannot find %s above %s; set NEO_ROOT to the repo or NEO_DAEMON_CMD to the daemon command", daemonEntry, real)
}

// daemonCommand is the command that starts the daemon: NEO_DAEMON_CMD, else
// bun running the entry point under NEO_ROOT or the repo found from exe.
func daemonCommand(getenv func(string) string, exe func() (string, error)) ([]string, error) {
	if c := strings.Fields(getenv("NEO_DAEMON_CMD")); len(c) > 0 {
		return c, nil
	}
	root, err := daemonRoot(getenv, exe)
	if err != nil {
		return nil, err
	}
	return []string{"bun", "run", filepath.Join(root, daemonEntry)}, nil
}

// daemonRoot is the repo the daemon runs from: NEO_ROOT, else found from exe.
func daemonRoot(getenv func(string) string, exe func() (string, error)) (string, error) {
	if root := getenv("NEO_ROOT"); root != "" {
		return root, nil
	}
	if exe == nil {
		exe = os.Executable
	}
	path, err := exe()
	if err != nil {
		return "", fmt.Errorf("cannot locate neo executable: %w", err)
	}
	return findRoot(path)
}

// restartIfStale stops a daemon started before its code last changed, unless a
// turn is running (then it only says so). It reports whether the daemon was
// stopped. A daemon run by NEO_DAEMON_CMD has no known sources: never stale.
func restartIfStale(ctx context.Context, socket string, e cliEnv) (bool, error) {
	if e.Getenv("NEO_DAEMON_CMD") != "" {
		return false, nil
	}
	root, err := daemonRoot(e.Getenv, e.Executable)
	if err != nil {
		return false, nil
	}
	st, err := inspect(ctx, socket)
	if err != nil || !newestSource(root).After(st.startedAt) {
		return false, nil
	}
	if st.busy {
		fmt.Fprintf(e.Stderr, "neo: the daemon (pid %d) is older than its code but a turn is running; run neo down and start again when it's done\n", st.pid)
		return false, nil
	}
	fmt.Fprintf(e.Stderr, "neo: the daemon (pid %d) is older than its code; restarting it\n", st.pid)
	if err := stopDaemon(ctx, socket, st.pid, e); err != nil {
		return false, err
	}
	return true, nil
}

// tailLines returns the last n lines of b.
func tailLines(b []byte, n int) string {
	lines := strings.Split(strings.TrimRight(string(b), "\n"), "\n")
	if len(lines) > n {
		lines = lines[len(lines)-n:]
	}
	return strings.Join(lines, "\n")
}

// ensureDaemon makes sure a daemon answers on socket. started is false if one
// already did. On failure the error says why (and includes the log tail).
func ensureDaemon(ctx context.Context, socket string, e cliEnv) (pid int, started bool, logPath string, err error) {
	if pid, err := probe(ctx, socket); err == nil {
		restarted, err := restartIfStale(ctx, socket, e)
		if err != nil {
			return 0, false, "", err
		}
		if !restarted {
			return pid, false, "", nil
		}
	} else if !errors.Is(err, sdk.ErrDaemonDown) {
		return 0, false, "", fmt.Errorf("cannot connect to daemon (socket %s): %w", socket, err)
	}

	argv, err := daemonCommand(e.Getenv, e.Executable)
	if err != nil {
		return 0, false, "", err
	}
	dir := dataDir(e.Getenv)
	if err := os.MkdirAll(dir, 0o700); err != nil {
		return 0, false, "", err
	}
	logPath = filepath.Join(dir, "daemon.log")
	logf, err := os.OpenFile(logPath, os.O_WRONLY|os.O_CREATE|os.O_APPEND, 0o600)
	if err != nil {
		return 0, false, "", err
	}
	defer logf.Close()
	offset, _ := logf.Seek(0, io.SeekEnd)
	devnull, err := os.Open(os.DevNull)
	if err != nil {
		return 0, false, "", err
	}
	defer devnull.Close()

	cmd := osexec.Command(argv[0], argv[1:]...)
	cmd.Stdin, cmd.Stdout, cmd.Stderr = devnull, logf, logf
	cmd.Env = append(e.environ(), "NEO_SOCKET="+socket)
	detach(cmd)
	if err := cmd.Start(); err != nil {
		return 0, false, logPath, fmt.Errorf("cannot start the daemon (%s): %w", strings.Join(argv, " "), err)
	}
	exited := make(chan error, 1)
	go func() { exited <- cmd.Wait() }()

	timeout := e.UpTimeout
	if timeout <= 0 {
		timeout = upTimeout
	}
	fail := func(format string, a ...any) error {
		tail := ""
		if data, err := os.ReadFile(logPath); err == nil && int64(len(data)) >= offset {
			tail = tailLines(data[offset:], logTailLines)
		}
		msg := fmt.Sprintf(format, a...) + " (log " + logPath + ")"
		if tail != "" {
			msg += ":\n" + tail
		}
		return errors.New(msg)
	}
	deadline := time.NewTimer(timeout)
	defer deadline.Stop()
	tick := time.NewTicker(pollEvery)
	defer tick.Stop()
	for {
		select {
		case werr := <-exited:
			// It may have exited after handing over to a daemon that answers.
			if pid, err := probe(ctx, socket); err == nil {
				return pid, true, logPath, nil
			}
			return 0, false, logPath, fail("daemon exited at startup (%v)", werr)
		case <-deadline.C:
			// Not answering: don't leave it running unseen.
			killGroup(cmd.Process.Pid)
			return 0, false, logPath, fail("daemon did not answer within %s", timeout)
		case <-ctx.Done():
			return 0, false, logPath, ctx.Err()
		case <-tick.C:
			if pid, err := probe(ctx, socket); err == nil {
				return pid, true, logPath, nil
			}
		}
	}
}

func runUp(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("up", e)
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	if len(positional) > 0 {
		fmt.Fprint(e.Stderr, "usage: neo up\n")
		return 2
	}
	path := sdk.SocketPath(*socket, e.Getenv, e.UID)
	pid, started, logPath, err := ensureDaemon(ctx, path, e)
	if err != nil {
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	if started {
		fmt.Fprintf(e.Stdout, "daemon started (pid %d, log %s)\n", pid, logPath)
	} else {
		fmt.Fprintf(e.Stdout, "daemon already running (pid %d, socket %s)\n", pid, path)
	}
	return 0
}

func runDown(ctx context.Context, args []string, e cliEnv) int {
	fs, socket := commandFlags("down", e)
	positional, err := parseFlags(fs, args)
	if err != nil {
		return 2
	}
	if len(positional) > 0 {
		fmt.Fprint(e.Stderr, "usage: neo down\n")
		return 2
	}
	path := sdk.SocketPath(*socket, e.Getenv, e.UID)
	pid, err := probe(ctx, path)
	if errors.Is(err, sdk.ErrDaemonDown) {
		fmt.Fprintln(e.Stdout, "daemon not running")
		return 0
	}
	if err != nil {
		fmt.Fprintf(e.Stderr, "neo: cannot connect to daemon (socket %s): %v\n", path, err)
		return 1
	}
	if pid <= 0 {
		fmt.Fprintf(e.Stderr, "neo: the daemon on %s reports no pid\n", path)
		return 1
	}
	if err := stopDaemon(ctx, path, pid, e); err != nil {
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	fmt.Fprintf(e.Stdout, "daemon stopped (pid %d)\n", pid)
	return 0
}

// stopDaemon terminates the daemon and waits until its socket goes quiet.
func stopDaemon(ctx context.Context, path string, pid int, e cliEnv) error {
	if err := terminate(pid); err != nil {
		return fmt.Errorf("cannot stop daemon (pid %d): %w", pid, err)
	}
	timeout := e.DownTimeout
	if timeout <= 0 {
		timeout = downTimeout
	}
	deadline := time.Now().Add(timeout)
	for {
		if _, err := probe(ctx, path); errors.Is(err, sdk.ErrDaemonDown) {
			return nil
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("daemon (pid %d) still answers on %s after %s", pid, path, timeout)
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(pollEvery):
		}
	}
}

// errTermMissing means the terminal app isn't built or installed.
var errTermMissing = errors.New("neo-term isn't built: run task build, or use neo attach")

// resolveTerm finds the terminal app: NEO_TERM, else neo-term next to the
// running executable, else on PATH.
func resolveTerm(getenv func(string) string, exe string, lookPath func(string) (string, error)) (string, error) {
	return resolveApp("NEO_TERM", "neo-term", errTermMissing, getenv, exe, lookPath)
}

// resolveApp finds an app binary: the path or command name in the environment variable envVar,
// else name next to the running executable (or its symlink target), else name on PATH.
func resolveApp(envVar, name string, missing error, getenv func(string) string, exe string, lookPath func(string) (string, error)) (string, error) {
	if t := getenv(envVar); t != "" {
		if strings.ContainsRune(t, '/') {
			if isExecutable(t) {
				return t, nil
			}
		} else if p, err := lookPath(t); err == nil {
			return p, nil
		}
		return "", fmt.Errorf("%s=%s: not found or not executable", envVar, t)
	}
	if exe != "" {
		dirs := []string{filepath.Dir(exe)}
		if real, err := filepath.EvalSymlinks(exe); err == nil && filepath.Dir(real) != dirs[0] {
			dirs = append(dirs, filepath.Dir(real))
		}
		for _, d := range dirs {
			if p := filepath.Join(d, name); isExecutable(p) {
				return p, nil
			}
		}
	}
	if p, err := lookPath(name); err == nil {
		return p, nil
	}
	return "", missing
}

func isExecutable(path string) bool {
	st, err := os.Stat(path)
	return err == nil && st.Mode().IsRegular() && st.Mode().Perm()&0o111 != 0
}

// runTerm is plain `neo` (or `neo DIR`) on a terminal: start the daemon if
// needed (saying so only then), then replace this process with the terminal
// app, given args.
func runTerm(ctx context.Context, args []string, e cliEnv) int {
	exe := ""
	if e.Executable != nil {
		exe, _ = e.Executable()
	} else {
		exe, _ = os.Executable()
	}
	lookPath := e.LookPath
	if lookPath == nil {
		lookPath = osexec.LookPath
	}
	// Resolve first so a missing app doesn't start a daemon for nothing.
	term, err := resolveTerm(e.Getenv, exe, lookPath)
	if err != nil {
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	path := sdk.SocketPath("", e.Getenv, e.UID)
	pid, started, logPath, err := ensureDaemon(ctx, path, e)
	if err != nil {
		fmt.Fprintf(e.Stderr, "neo: %v\n", err)
		return 1
	}
	if started {
		fmt.Fprintf(e.Stdout, "daemon started (pid %d, log %s)\n", pid, logPath)
	}
	execFn := e.Exec
	if execFn == nil {
		execFn = execReplace
	}
	err = execFn(term, append([]string{term}, args...), e.environ())
	fmt.Fprintf(e.Stderr, "neo: cannot run %s: %v\n", term, err)
	return 1
}

// environ is the environment for child processes.
func (e cliEnv) environ() []string {
	if e.Environ != nil {
		return e.Environ()
	}
	return os.Environ()
}
