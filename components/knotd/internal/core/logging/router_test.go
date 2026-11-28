package logging

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"strings"
	"sync"
	"testing"
	"time"
)

var zeroTime time.Time

type testHandler struct {
	records []slog.Record
	enabled bool
	err     error
}

func (h *testHandler) Enabled(
	_ context.Context,
	_ slog.Level,
) bool {
	return h.enabled
}

func (h *testHandler) Handle(
	_ context.Context,
	record slog.Record,
) error {
	if h.err != nil {
		return h.err
	}

	h.records = append(h.records, record.Clone())
	return nil
}

func (h *testHandler) WithAttrs(attrs []slog.Attr) slog.Handler {
	clone := *h

	for i := range clone.records {
		clone.records[i] = clone.records[i].Clone()
	}

	clone.records = append(clone.records, make([]slog.Record, 0)...)
	return &clone
}

func (h *testHandler) WithGroup(_ string) slog.Handler {
	clone := *h
	return &clone
}

func newJSONHandler(buf *bytes.Buffer) slog.Handler {
	return slog.NewJSONHandler(buf, &slog.HandlerOptions{
		Level: slog.LevelDebug,
	})
}

func decodeLogs(t *testing.T, buf *bytes.Buffer) []map[string]any {
	t.Helper()

	var logs []map[string]any

	decoder := json.NewDecoder(bytes.NewReader(buf.Bytes()))

	for decoder.More() {
		var entry map[string]any

		if err := decoder.Decode(&entry); err != nil {
			t.Fatalf("decoding log record: %v", err)
		}

		logs = append(logs, entry)
	}

	return logs
}

func TestLogRouter_RoutesBelowThresholdToPrimary(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	logger := slog.New(router)

	logger.Info("info message")

	primaryLogs := decodeLogs(t, &primaryBuf)
	secondaryLogs := decodeLogs(t, &secondaryBuf)

	if len(primaryLogs) != 1 {
		t.Fatalf("primary logs = %d, want 1", len(primaryLogs))
	}

	if len(secondaryLogs) != 0 {
		t.Fatalf("secondary logs = %d, want 0", len(secondaryLogs))
	}
}

func TestLogRouter_RoutesAtThresholdToSecondary(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	logger := slog.New(router)

	logger.Warn("warning message")

	if got := decodeLogs(t, &primaryBuf); len(got) != 1 {
		t.Fatalf("primary logs = %d, want 1", len(got))
	}

	secondaryLogs := decodeLogs(t, &secondaryBuf)

	if len(secondaryLogs) != 1 {
		t.Fatalf("secondary logs = %d, want 1", len(secondaryLogs))
	}
}

func TestLogRouter_RoutesAboveThresholdToSecondary(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	logger := slog.New(router)

	logger.Error("error message")

	if got := decodeLogs(t, &primaryBuf); len(got) != 1 {
		t.Fatalf("primary logs = %d, want 1", len(got))
	}

	if got := decodeLogs(t, &secondaryBuf); len(got) != 1 {
		t.Fatalf("secondary logs = %d, want 1", len(got))
	}
}

func TestLogRouter_RoutesAllStandardLevels(t *testing.T) {
	tests := []struct {
		name       string
		level      slog.Level
		wantTarget string
	}{
		{"debug", slog.LevelDebug, "primary"},
		{"info", slog.LevelInfo, "primary"},
		{"warn", slog.LevelWarn, "secondary"},
		{"error", slog.LevelError, "secondary"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var primaryBuf, secondaryBuf bytes.Buffer

			router := NewLogRouter(
				newJSONHandler(&primaryBuf),
				newJSONHandler(&secondaryBuf),
				slog.LevelWarn,
			)

			logger := slog.New(router)
			logger.Log(context.Background(), tt.level, tt.name)

			primaryLogs := decodeLogs(t, &primaryBuf)
			secondaryLogs := decodeLogs(t, &secondaryBuf)

			switch tt.wantTarget {
			case "primary":
				if len(primaryLogs) != 1 || len(secondaryLogs) != 0 {
					t.Fatalf(
						"unexpected routing: primary=%d secondary=%d",
						len(primaryLogs),
						len(secondaryLogs),
					)
				}
			case "secondary":
				if len(primaryLogs) != 1 || len(secondaryLogs) != 1 {
					t.Fatalf(
						"unexpected routing: primary=%d secondary=%d",
						len(primaryLogs),
						len(secondaryLogs),
					)
				}
			}
		})
	}
}

func TestLogRouter_Enabled(t *testing.T) {
	tests := []struct {
		name      string
		level     slog.Level
		primary   bool
		secondary bool
		want      bool
	}{
		{
			name:      "below threshold - primary enabled",
			level:     slog.LevelInfo,
			primary:   true,
			secondary: false,
			want:      true,
		},
		{
			name:      "below threshold - primary disabled",
			level:     slog.LevelInfo,
			primary:   false,
			secondary: true,
			want:      false,
		},
		{
			name:      "at threshold - secondary enabled",
			level:     slog.LevelWarn,
			primary:   false,
			secondary: true,
			want:      true,
		},
		{
			name:      "at threshold - secondary disabled",
			level:     slog.LevelWarn,
			primary:   true,
			secondary: false,
			want:      true,
		},
		{
			name:      "above threshold - secondary enabled",
			level:     slog.LevelError,
			primary:   false,
			secondary: true,
			want:      true,
		},
		{
			name:      "both disabled",
			level:     slog.LevelInfo,
			primary:   false,
			secondary: false,
			want:      false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			router := NewLogRouter(
				&testHandler{enabled: tt.primary},
				&testHandler{enabled: tt.secondary},
				slog.LevelWarn,
			)

			got := router.Enabled(context.Background(), tt.level)

			if got != tt.want {
				t.Fatalf("Enabled() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestLogRouter_HandlePropagatesError(t *testing.T) {
	expectedErr := errors.New("handler failure")

	primary := &testHandler{
		enabled: true,
		err:     expectedErr,
	}

	secondary := &testHandler{
		enabled: true,
	}

	router := NewLogRouter(primary, secondary, slog.LevelWarn)

	record := slog.NewRecord(
		// Time is irrelevant for this test.
		// Zero time is valid.
		zeroTime,
		slog.LevelInfo,
		"test",
		0,
	)

	err := router.Handle(context.Background(), record)

	if !errors.Is(err, expectedErr) {
		t.Fatalf("Handle() error = %v, want %v", err, expectedErr)
	}
}

func TestLogRouter_PreservesAttributes(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	logger := slog.New(router).With(
		slog.String("service", "knotd"),
		slog.Int("version", 1),
	)

	logger.Info("started")

	logs := decodeLogs(t, &primaryBuf)

	if len(logs) != 1 {
		t.Fatalf("logs = %d, want 1", len(logs))
	}

	if got := logs[0]["service"]; got != "knotd" {
		t.Fatalf("service = %v, want knotd", got)
	}

	if got := logs[0]["version"]; got != float64(1) {
		t.Fatalf("version = %v, want 1", got)
	}
}

func TestLogRouter_PreservesGroups(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	logger := slog.New(router).
		WithGroup("daemon").
		With(slog.String("name", "knotd"))

	logger.Info("started")

	logs := decodeLogs(t, &primaryBuf)

	if len(logs) != 1 {
		t.Fatalf("logs = %d, want 1", len(logs))
	}

	group, ok := logs[0]["daemon"].(map[string]any)
	if !ok {
		t.Fatalf("daemon group missing or invalid: %#v", logs[0]["daemon"])
	}

	if got := group["name"]; got != "knotd" {
		t.Fatalf("daemon.name = %v, want knotd", got)
	}
}

func TestLogRouter_DoesNotModifyOriginalRecord(t *testing.T) {
	primary := &testHandler{enabled: true}
	secondary := &testHandler{enabled: true}

	router := NewLogRouter(primary, secondary, slog.LevelWarn)

	record := slog.NewRecord(
		zeroTime,
		slog.LevelInfo,
		"original",
		0,
	)

	record.AddAttrs(slog.String("key", "value"))

	err := router.Handle(context.Background(), record)
	if err != nil {
		t.Fatalf("Handle() error = %v", err)
	}

	if record.Message != "original" {
		t.Fatalf("original message modified: %q", record.Message)
	}

	if record.NumAttrs() != 1 {
		t.Fatalf("original attributes modified: %d", record.NumAttrs())
	}
}

func TestLogRouter_CustomLevel(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	threshold := slog.Level(5)

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		threshold,
	)

	logger := slog.New(router)

	logger.Log(context.Background(), slog.Level(4), "below")
	logger.Log(context.Background(), slog.Level(5), "equal")
	logger.Log(context.Background(), slog.Level(6), "above")

	if got := len(decodeLogs(t, &primaryBuf)); got != 3 {
		t.Fatalf("primary logs = %d, want 3", got)
	}

	if got := len(decodeLogs(t, &secondaryBuf)); got != 2 {
		t.Fatalf("secondary logs = %d, want 2", got)
	}
}

func TestLogRouter_GlobalDefault(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	previous := slog.Default()
	t.Cleanup(func() {
		slog.SetDefault(previous)
	})

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	slog.SetDefault(slog.New(router))

	slog.Info("global info")
	slog.Error("global error")

	if got := len(decodeLogs(t, &primaryBuf)); got != 2 {
		t.Fatalf("primary logs = %d, want 2", got)
	}

	if got := len(decodeLogs(t, &secondaryBuf)); got != 1 {
		t.Fatalf("secondary logs = %d, want 1", got)
	}
}

func TestLogRouter_RoutesToPrimaryAndSecondary(t *testing.T) {
	tests := []struct {
		name          string
		level         slog.Level
		wantPrimary   int
		wantSecondary int
	}{
		{
			name:          "below threshold",
			level:         slog.LevelInfo,
			wantPrimary:   1,
			wantSecondary: 0,
		},
		{
			name:          "at threshold",
			level:         slog.LevelWarn,
			wantPrimary:   1,
			wantSecondary: 1,
		},
		{
			name:          "above threshold",
			level:         slog.LevelError,
			wantPrimary:   1,
			wantSecondary: 1,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			primary := &testHandler{enabled: true}
			secondary := &testHandler{enabled: true}

			router := NewLogRouter(primary, secondary, slog.LevelWarn)

			record := slog.NewRecord(
				time.Time{},
				tt.level,
				"test message",
				0,
			)

			if err := router.Handle(context.Background(), record); err != nil {
				t.Fatalf("Handle() error = %v", err)
			}

			if got := len(primary.records); got != tt.wantPrimary {
				t.Errorf("primary records = %d, want %d", got, tt.wantPrimary)
			}

			if got := len(secondary.records); got != tt.wantSecondary {
				t.Errorf("secondary records = %d, want %d", got, tt.wantSecondary)
			}
		})
	}
}

func TestLogRouter_PrimaryFailureDoesNotPreventSecondary(t *testing.T) {
	primaryErr := errors.New("primary failure")

	primary := &testHandler{
		enabled: true,
		err:     primaryErr,
	}

	secondary := &testHandler{
		enabled: true,
	}

	router := NewLogRouter(primary, secondary, slog.LevelWarn)

	record := slog.NewRecord(
		time.Time{},
		slog.LevelError,
		"test",
		0,
	)

	err := router.Handle(context.Background(), record)

	if !errors.Is(err, primaryErr) {
		t.Fatalf("Handle() error = %v, want primary error", err)
	}

	if len(secondary.records) != 1 {
		t.Fatalf("secondary records = %d, want 1", len(secondary.records))
	}
}

func TestLogRouter_SecondaryFailureDoesNotPreventPrimary(t *testing.T) {
	secondaryErr := errors.New("secondary failure")

	primary := &testHandler{
		enabled: true,
	}

	secondary := &testHandler{
		enabled: true,
		err:     secondaryErr,
	}

	router := NewLogRouter(primary, secondary, slog.LevelWarn)

	record := slog.NewRecord(
		time.Time{},
		slog.LevelError,
		"test",
		0,
	)

	err := router.Handle(context.Background(), record)

	if !errors.Is(err, secondaryErr) {
		t.Fatalf("Handle() error = %v, want secondary error", err)
	}

	if len(primary.records) != 1 {
		t.Fatalf("primary records = %d, want 1", len(primary.records))
	}
}

func TestLogRouter_JoinsHandlerErrors(t *testing.T) {
	primaryErr := errors.New("primary failure")
	secondaryErr := errors.New("secondary failure")

	router := NewLogRouter(
		&testHandler{enabled: true, err: primaryErr},
		&testHandler{enabled: true, err: secondaryErr},
		slog.LevelWarn,
	)

	record := slog.NewRecord(
		time.Time{},
		slog.LevelError,
		"test",
		0,
	)

	err := router.Handle(context.Background(), record)

	if !errors.Is(err, primaryErr) {
		t.Error("expected primary error to be preserved")
	}

	if !errors.Is(err, secondaryErr) {
		t.Error("expected secondary error to be preserved")
	}
}

func TestLogRouter_MessagePreserved(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	slog.New(router).Info("hello world")

	logs := decodeLogs(t, &primaryBuf)

	if len(logs) != 1 {
		t.Fatalf("logs = %d, want 1", len(logs))
	}

	if got := logs[0]["msg"]; got != "hello world" {
		t.Fatalf("message = %v, want hello world", got)
	}
}

func TestLogRouter_ConcurrentLogging(t *testing.T) {
	var primaryBuf, secondaryBuf bytes.Buffer

	router := NewLogRouter(
		newJSONHandler(&primaryBuf),
		newJSONHandler(&secondaryBuf),
		slog.LevelWarn,
	)

	logger := slog.New(router)

	const workers = 16
	const messagesPerWorker = 100

	var wg sync.WaitGroup

	for worker := 0; worker < workers; worker++ {
		wg.Add(1)

		go func(worker int) {
			defer wg.Done()

			for i := 0; i < messagesPerWorker; i++ {
				logger.Info("message",
					slog.Int("worker", worker),
					slog.Int("sequence", i),
				)

				logger.Error("error",
					slog.Int("worker", worker),
					slog.Int("sequence", i),
				)
			}
		}(worker)
	}

	wg.Wait()

	want := workers * messagesPerWorker

	if got := strings.Count(primaryBuf.String(), "\n"); got != want*2 {
		t.Fatalf("primary records = %d, want %d", got, want)
	}

	if got := strings.Count(secondaryBuf.String(), "\n"); got != want {
		t.Fatalf("secondary records = %d, want %d", got, want)
	}
}

func TestLogRouter_DoesNotDuplicatePrimaryRecords(t *testing.T) {
	primary := &testHandler{enabled: true}
	secondary := &testHandler{enabled: true}

	router := NewLogRouter(primary, secondary, slog.LevelWarn)

	logger := slog.New(router)
	logger.Error("critical failure")

	if got := len(primary.records); got != 1 {
		t.Fatalf("primary records = %d, want 1", got)
	}

	if got := len(secondary.records); got != 1 {
		t.Fatalf("secondary records = %d, want 1", got)
	}
}
