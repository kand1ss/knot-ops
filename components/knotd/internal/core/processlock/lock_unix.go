//go:build unix

package processlock

import (
	"errors"
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

func lockFile(file *os.File) error {
	err := unix.Flock(
		int(file.Fd()),
		unix.LOCK_EX|unix.LOCK_NB,
	)
	if err != nil {
		if errors.Is(err, unix.EWOULDBLOCK) ||
			errors.Is(err, unix.EAGAIN) {
			return fmt.Errorf("%w: %s", ErrAlreadyLocked, file.Name())
		}

		return fmt.Errorf("flock: %w", err)
	}

	return nil
}

func unlockFile(file *os.File) error {
	if err := unix.Flock(
		int(file.Fd()),
		unix.LOCK_UN,
	); err != nil {
		return fmt.Errorf("flock unlock: %w", err)
	}

	return nil
}
