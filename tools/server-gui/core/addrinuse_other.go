//go:build !windows

package core

import (
	"errors"
	"syscall"
)

// IsAddrInUse reports whether err means the port is already taken.
func IsAddrInUse(err error) bool {
	return errors.Is(err, syscall.EADDRINUSE)
}
