//go:build windows

package processlock

import (
	"errors"
	"os"

	"golang.org/x/sys/windows"
)

// lockOffset places the locked byte far beyond any real file content so the
// PID region stays readable by other handles (client reads PID while the
// daemon holds the lock). LockFileEx permits locking past EOF.
const (
	lockOffsetHigh uint32 = 0x100 // offset = 0x100 << 32 = 1 TiB
	lockOffsetLow  uint32 = 0
	lockLenLow     uint32 = 1
	lockLenHigh    uint32 = 0
)

func newOverlapped() *windows.Overlapped {
	return &windows.Overlapped{
		Offset:     lockOffsetLow,
		OffsetHigh: lockOffsetHigh,
	}
}

func lockFile(f *os.File) error {
	err := windows.LockFileEx(
		windows.Handle(f.Fd()),
		windows.LOCKFILE_EXCLUSIVE_LOCK|windows.LOCKFILE_FAIL_IMMEDIATELY,
		0, // reserved
		lockLenLow,
		lockLenHigh,
		newOverlapped(),
	)
	if err != nil {
		if errors.Is(err, windows.ERROR_LOCK_VIOLATION) {
			return ErrAlreadyLocked
		}
		return err
	}
	return nil
}

func unlockFile(f *os.File) error {
	err := windows.UnlockFileEx(
		windows.Handle(f.Fd()),
		0, // reserved
		lockLenLow,
		lockLenHigh,
		newOverlapped(),
	)
	if err != nil && !errors.Is(err, windows.ERROR_NOT_LOCKED) {
		return err
	}
	return nil
}
