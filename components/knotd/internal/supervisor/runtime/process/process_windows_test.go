//go:build windows

package process

import (
	"os"
	"path/filepath"
	"syscall"
	"testing"

	"golang.org/x/sys/windows"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

func TestBuildCommand_Windows_QuotedPathAndSuspendedRegression(t *testing.T) {
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
		Name:      "test-service",
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
		t.Errorf("expected cmd.Args to be empty for raw CmdLine execution, got: %v", cmd.Args)
	}

	expectedFlags := uint32(syscall.CREATE_NEW_PROCESS_GROUP | windows.CREATE_SUSPENDED)
	if cmd.SysProcAttr.CreationFlags != expectedFlags {
		t.Errorf("creation flags mismatch: got %x, want %x", cmd.SysProcAttr.CreationFlags, expectedFlags)
	}

	if err := cmd.Start(); err != nil {
		t.Fatalf("failed starting suspended process: %v", err)
	}

	defer func() {
		_ = cmd.Process.Kill()
		_, _ = cmd.Process.Wait()
	}()

	if _, err := os.Stat(pidFile); !os.IsNotExist(err) {
		t.Fatalf("PID_FILE was created before resuming thread - process was not properly suspended")
	}

	if err := resumeProcess(cmd.Process.Pid); err != nil {
		t.Fatalf("failed to resume process: %v", err)
	}

	if err := cmd.Wait(); err != nil {
		t.Fatalf("command failed after resume (exit code 1 regression): %v", err)
	}

	if _, err := os.Stat(pidFile); os.IsNotExist(err) {
		t.Errorf("PID_FILE was not created, script did not run properly after resume")
	}
}
