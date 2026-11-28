package supervisor

import (
	"log/slog"
	"os"
	"testing"
)

func TestMain(m *testing.M) {
	if os.Getenv("KNOT_TEST_LOG") == "" {
		slog.SetDefault(slog.New(slog.DiscardHandler))
	}
	os.Exit(m.Run())
}
