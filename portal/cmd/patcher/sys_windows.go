package main

import (
	"errors"
	"syscall"
)

// isSharingViolation is true for the errors Windows gives when another
// program has the file open.
func isSharingViolation(err error) bool {
	var errno syscall.Errno
	if !errors.As(err, &errno) {
		return false
	}
	const errorSharingViolation, errorLockViolation = 32, 33
	return errno == syscall.ERROR_ACCESS_DENIED || errno == errorSharingViolation || errno == errorLockViolation
}
