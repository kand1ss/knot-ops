package processlock

import (
	"errors"
	"fmt"
	"os"
	"sync"
)

var ErrAlreadyLocked = errors.New("file is already locked")

type FileLock struct {
	file *os.File

	closeOnce sync.Once
	closeErr  error
}

func Acquire(path string) (*FileLock, error) {
	if path == "" {
		return nil, errors.New("lock path cannot be empty")
	}

	file, err := os.OpenFile(
		path,
		os.O_CREATE|os.O_RDWR,
		0600,
	)
	if err != nil {
		return nil, fmt.Errorf("open lock file: %w", err)
	}

	if err := lockFile(file); err != nil {
		_ = file.Close()
		return nil, fmt.Errorf("acquire lock: %w", err)
	}

	return &FileLock{file: file}, nil
}

func (l *FileLock) Close() error {
	if l == nil {
		return nil
	}

	l.closeOnce.Do(func() {
		unlockErr := unlockFile(l.file)
		closeErr := l.file.Close()

		l.closeErr = errors.Join(
			wrapError("unlock file", unlockErr),
			wrapError("close file", closeErr),
		)
	})

	return l.closeErr
}

func wrapError(operation string, err error) error {
	if err == nil {
		return nil
	}

	return fmt.Errorf("%s: %w", operation, err)
}
