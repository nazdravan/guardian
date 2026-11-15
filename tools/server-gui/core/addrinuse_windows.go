//go:build windows

package core

import (
	"errors"
	"syscall"
)

// wsaeaddrinuse is WSAEADDRINUSE, which package syscall does not name.
const wsaeaddrinuse = syscall.Errno(10048)

// IsAddrInUse reports whether err means the port is already taken.
func IsAddrInUse(err error) bool {
	return errors.Is(err, wsaeaddrinuse) || errors.Is(err, syscall.EADDRINUSE)
}
