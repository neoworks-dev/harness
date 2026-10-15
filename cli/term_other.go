//go:build !unix

package main

import "os"

// terminalSize is unknown here; callers fall back to $COLUMNS or 80.
func terminalSize(*os.File) (cols, rows int) { return 0, 0 }
