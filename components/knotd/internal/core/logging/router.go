package logging

import (
	"context"
	"errors"
	"log/slog"
)

// LogRouter fans a record out to two handlers with independent filtering:
//   - primary receives every record accepted by its own Enabled();
//   - secondary additionally requires level >= threshold.
//
// Invariant: primary and secondary are never nil (use NewLogRouter, which
// enforces this). For single-sink setups, skip the router entirely.
type LogRouter struct {
	primary   slog.Handler
	secondary slog.Handler
	threshold slog.Level
}

func NewLogRouter(primary, secondary slog.Handler, threshold slog.Level) *LogRouter {
	if primary == nil || secondary == nil {
		// Programmer error: fail at construction, not on the first log line.
		panic("logging: NewLogRouter requires non-nil handlers")
	}
	return &LogRouter{primary: primary, secondary: secondary, threshold: threshold}
}

func (r *LogRouter) Enabled(ctx context.Context, level slog.Level) bool {
	return r.primary.Enabled(ctx, level) ||
		(level >= r.threshold && r.secondary.Enabled(ctx, level))
}

func (r *LogRouter) Handle(ctx context.Context, record slog.Record) error {
	usePrimary := r.primary.Enabled(ctx, record.Level)
	useSecondary := record.Level >= r.threshold && r.secondary.Enabled(ctx, record.Level)

	var primaryErr, secondaryErr error
	if usePrimary {
		// Clone only when both sinks fire: slog.Record shares its backing
		// attr slice, so a handler that mutates it would corrupt the other.
		rec := record
		if useSecondary {
			rec = record.Clone()
		}
		primaryErr = r.primary.Handle(ctx, rec)
	}
	if useSecondary {
		secondaryErr = r.secondary.Handle(ctx, record)
	}
	return errors.Join(primaryErr, secondaryErr)
}

func (r *LogRouter) WithAttrs(attrs []slog.Attr) slog.Handler {
	return &LogRouter{
		primary:   r.primary.WithAttrs(attrs),
		secondary: r.secondary.WithAttrs(attrs),
		threshold: r.threshold,
	}
}

func (r *LogRouter) WithGroup(name string) slog.Handler {
	return &LogRouter{
		primary:   r.primary.WithGroup(name),
		secondary: r.secondary.WithGroup(name),
		threshold: r.threshold,
	}
}
