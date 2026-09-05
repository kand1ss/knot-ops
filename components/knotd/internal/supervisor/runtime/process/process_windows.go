//go:build windows

package process

import (
	"errors"
	"os"
	"os/exec"
	"syscall"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

func buildCommand(service domain.ServiceSpec) *exec.Cmd {
	cmd := exec.Command("cmd", "/C", service.Command)
	cmd.Dir = service.Directory
	cmd.Env = mergeEnv(os.Environ(), service.Env)

	cmd.SysProcAttr = &syscall.SysProcAttr{
		CreationFlags: syscall.CREATE_NEW_PROCESS_GROUP,
	}

	cmd.Stdout = nil
	cmd.Stderr = nil
	cmd.Stdin = nil

	return cmd
}

// terminateGraceful: Windows has no SIGTERM reachable from a Go process
// belonging to a different console session, and GenerateConsoleCtrlEvent
// against a "cmd /C"-spawned tree is unreliable in practice. This is
// force-kill-only on Windows until a proper CTRL_BREAK_EVENT path is
// built — stated honestly rather than faked as graceful.
func terminateGraceful(proc *os.Process) error {
	return proc.Kill()
}

func killForceful(proc *os.Process) error {
	return proc.Kill()
}

func isProcessDone(err error) bool {
	return errors.Is(err, os.ErrProcessDone)
}
