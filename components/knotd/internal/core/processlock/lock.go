package processlock

import (
	"errors"
	"fmt"
	"os"
	"strconv"
	"sync"
)

var (
	ErrAlreadyLocked = errors.New("file is already locked")
	ErrClosed        = errors.New("lock is closed")
)

type FileLock struct {
	file *os.File

	mu     sync.Mutex // guards file I/O vs Close
	closed bool

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

// Write replaces the lock file contents with data. It writes through the
// descriptor that holds the lock (required on Windows, where other handles
// cannot write to a locked region).
//
// The new content is written before truncation so a concurrent reader never
// observes an empty file.
func (l *FileLock) Write(data []byte) error {
	if l == nil {
		return ErrClosed
	}

	l.mu.Lock()
	defer l.mu.Unlock()

	if l.closed {
		return ErrClosed
	}

	if _, err := l.file.WriteAt(data, 0); err != nil {
		return fmt.Errorf("write lock file: %w", err)
	}
	if err := l.file.Truncate(int64(len(data))); err != nil {
		return fmt.Errorf("truncate lock file: %w", err)
	}
	if err := l.file.Sync(); err != nil {
		return fmt.Errorf("sync lock file: %w", err)
	}
	return nil
}

// WritePID stores the current process ID in the lock file.
// Format: decimal ASCII + "\n", parsed by the client after trimming.
func (l *FileLock) WritePID() error {
	return l.Write([]byte(strconv.Itoa(os.Getpid()) + "\n"))
}

func (l *FileLock) Close() error {
	if l == nil {
		return nil
	}

	l.closeOnce.Do(func() {
		l.mu.Lock()
		defer l.mu.Unlock()
		l.closed = true

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
