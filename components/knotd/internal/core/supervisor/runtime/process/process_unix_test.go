//go:build !windows

package process

import (
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

func TestBuildCommand_Unix_EmptyArgsRegression(t *testing.T) {
	tests := []struct {
		name    string
		command string
	}{
		{
			name:    "empty command string",
			command: "",
		},
		{
			name:    "whitespace only command",
			command: "   ",
		},
		{
			name:    "tabs and newlines only",
			command: "\t\n  \r",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			svc := domain.ServiceSpec{
				Name:    "test-service",
				Command: tt.command,
			}

			cmd, err := buildCommand(svc)
			if err == nil {
				t.Fatalf("expected error when len(args) == 0 for command %q, got nil (cmd: %v)", tt.command, cmd)
			}

			if cmd != nil {
				t.Errorf("expected cmd to be nil on error, got: %v", cmd)
			}
		})
	}
}

func TestBuildCommand_Unix_InvalidSyntax(t *testing.T) {
	svc := domain.ServiceSpec{
		Name:    "test-service",
		Command: `echo "unclosed string`,
	}

	cmd, err := buildCommand(svc)
	if err == nil {
		t.Fatalf("expected shellquote parsing error, got nil")
	}
	if cmd != nil {
		t.Errorf("expected cmd to be nil on parse error, got: %v", cmd)
	}
}

func TestBuildCommand_Unix_Success(t *testing.T) {
	svc := domain.ServiceSpec{
		Name:      "test-service",
		Command:   `sh -c "echo hello"`,
		Directory: "/tmp",
		Env:       map[string]string{"FOO": "BAR"},
	}

	cmd, err := buildCommand(svc)
	if err != nil {
		t.Fatalf("unexpected error building command: %v", err)
	}

	// 1. Проверяем точный разбор аргументов через shellquote
	expectedArgs := []string{"sh", "-c", "echo hello"}
	if len(cmd.Args) != len(expectedArgs) {
		t.Fatalf("args length mismatch: got %d, want %d", len(cmd.Args), len(expectedArgs))
	}
	for i := range expectedArgs {
		if cmd.Args[i] != expectedArgs[i] {
			t.Errorf("arg[%d] mismatch: got %q, want %q", i, cmd.Args[i], expectedArgs[i])
		}
	}

	// 2. Проверяем системные атрибуты и окружение
	if cmd.Dir != "/tmp" {
		t.Errorf("unexpected directory: got %q, want %q", cmd.Dir, "/tmp")
	}

	if cmd.SysProcAttr == nil || !cmd.SysProcAttr.Setpgid {
		t.Errorf("expected SysProcAttr.Setpgid to be true for process group isolation")
	}
}
