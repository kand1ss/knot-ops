//go:build windows

package processlock

import (
	"errors"
	"fmt"
	"os"

	"golang.org/x/sys/windows"
)

const (
	lockFlags = windows.LOCKFILE_EXCLUSIVE_LOCK |
		windows.LOCKFILE_FAIL_IMMEDIATELY

	lockRangeLow  = ^uint32(0)
	lockRangeHigh = ^uint32(0)
)

func lockFile(file *os.File) error {
	overlapped := &windows.Overlapped{}

	err := windows.LockFileEx(
		windows.Handle(file.Fd()),
		lockFlags,
		0,
		lockRangeLow,
		lockRangeHigh,
		overlapped,
	)
	if err != nil {
		if errors.Is(err, windows.ERROR_LOCK_VIOLATION) {
			return fmt.Errorf("%w: %s", ErrAlreadyLocked, file.Name())
		}

		return fmt.Errorf("LockFileEx: %w", err)
	}

	return nil
}

func unlockFile(file *os.File) error {
	overlapped := &windows.Overlapped{}

	err := windows.UnlockFileEx(
		windows.Handle(file.Fd()),
		0,
		lockRangeLow,
		lockRangeHigh,
		overlapped,
	)
	if err != nil {
		return fmt.Errorf("UnlockFileEx: %w", err)
	}

	return nil
}
