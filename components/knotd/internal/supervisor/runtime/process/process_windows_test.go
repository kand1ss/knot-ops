package process

import (
	"os"
	"path/filepath"
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

func TestBuildCommand_WindowsQuotedPathRegression(t *testing.T) {
	tmpDir := t.TempDir()

	dirWithSpaces := filepath.Join(tmpDir, "path with spaces")
	if err := os.MkdirAll(dirWithSpaces, 0755); err != nil {
		t.Fatalf("failed to create dir with spaces: %v", err)
	}

	scriptPath := filepath.Join(dirWithSpaces, "helper script.bat")
	pidFile := filepath.Join(tmpDir, "PID_FILE")

	scriptContent := `@echo off
if "%~1"=="hello world" (
    echo OK > "` + pidFile + `"
)
`
	if err := os.WriteFile(scriptPath, []byte(scriptContent), 0755); err != nil {
		t.Fatalf("failed to write helper script: %v", err)
	}

	svcCommand := `"` + scriptPath + `" "hello world"`

	svc := domain.ServiceSpec{
		Command:   svcCommand,
		Directory: tmpDir,
	}

	cmd, err := buildCommand(svc)
	if err != nil {
		t.Fatalf("buildCommand failed: %v", err)
	}

	expectedCmdLine := `cmd.exe /S /C "` + svcCommand + `"`
	if cmd.SysProcAttr == nil || cmd.SysProcAttr.CmdLine != expectedCmdLine {
		t.Errorf("unexpected CmdLine.\nGot:  %q\nWant: %q", cmd.SysProcAttr.CmdLine, expectedCmdLine)
	}

	if len(cmd.Args) != 0 {
		t.Errorf("expected cmd.Args to be empty, got: %v", cmd.Args)
	}

	if err := cmd.Run(); err != nil {
		t.Fatalf("command failed to execute (exit code 1 regression): %v", err)
	}

	if _, err := os.Stat(pidFile); os.IsNotExist(err) {
		t.Errorf("PID_FILE was not created, script did not run properly")
	}
}
