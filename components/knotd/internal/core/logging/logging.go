package logging

import (
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"time"

	"gopkg.in/natefinch/lumberjack.v2"
)

type Config struct {
	// Path is the full path of the active log file. The caller owns the
	// naming; rotated backups are created next to it by lumberjack.
	Path string

	// FileLevel gates the file sink. Mutable at runtime.
	FileLevel *slog.LevelVar

	// Foreground mirrors records to stderr (text). False in detached (-d) mode.
	Foreground bool
	// ConsoleThreshold is the minimum level that reaches stderr.
	// Ignored when Foreground is false.
	ConsoleThreshold slog.Level

	MaxSizeMB  int
	MaxBackups int
	MaxAgeDays int
}

func (c *Config) validate() error {
	if c.Path == "" {
		return fmt.Errorf("log path must not be empty")
	}
	if strings.HasSuffix(c.Path, string(os.PathSeparator)) || strings.HasSuffix(c.Path, "/") {
		return fmt.Errorf("log path %q points to a directory, expected a file", c.Path)
	}
	abs, err := filepath.Abs(c.Path) // immune to later chdir (daemonization)
	if err != nil {
		return fmt.Errorf("resolve log path: %w", err)
	}
	c.Path = abs

	if c.FileLevel == nil {
		return fmt.Errorf("FileLevel must not be nil")
	}
	if c.MaxSizeMB <= 0 {
		c.MaxSizeMB = 50
	}
	if c.MaxBackups <= 0 {
		c.MaxBackups = 5
	}
	if c.MaxAgeDays <= 0 {
		c.MaxAgeDays = 14
	}
	return nil
}

// normalizeAttrs: single place that defines the on-disk/console schema.
// Key "time" -> "ts", always UTC RFC3339Nano.
func normalizeAttrs(_ []string, a slog.Attr) slog.Attr {
	if a.Key == slog.TimeKey && a.Value.Kind() == slog.KindTime {
		a.Key = "ts"
		a.Value = slog.StringValue(a.Value.Time().UTC().Format(time.RFC3339Nano))
	}
	return a
}

// New builds the daemon logger. The returned io.Closer MUST be closed on
// shutdown (flushes the file; mandatory on Windows to release the handle).
func New(cfg Config) (*slog.Logger, io.Closer, error) {
	if err := cfg.validate(); err != nil {
		return nil, nil, fmt.Errorf("invalid logging config: %w", err)
	}
	if err := os.MkdirAll(filepath.Dir(cfg.Path), 0o700); err != nil {
		return nil, nil, fmt.Errorf("create log dir: %w", err)
	}

	file := &lumberjack.Logger{
		Filename:   cfg.Path,
		MaxSize:    cfg.MaxSizeMB,
		MaxBackups: cfg.MaxBackups,
		MaxAge:     cfg.MaxAgeDays,
		Compress:   true,
	}

	fileHandler := slog.NewJSONHandler(file, &slog.HandlerOptions{
		Level:       cfg.FileLevel,
		ReplaceAttr: normalizeAttrs,
	})

	// Detached mode: single sink, no router, no stderr.
	if !cfg.Foreground {
		return slog.New(fileHandler), file, nil
	}

	// Console handler is intentionally unfiltered (Debug floor):
	// the router's threshold is the single source of truth for console level.
	consoleHandler := slog.NewTextHandler(os.Stderr, &slog.HandlerOptions{
		Level:       slog.LevelDebug,
		ReplaceAttr: normalizeAttrs,
	})

	router := NewLogRouter(fileHandler, consoleHandler, cfg.ConsoleThreshold)
	return slog.New(router), file, nil
}
