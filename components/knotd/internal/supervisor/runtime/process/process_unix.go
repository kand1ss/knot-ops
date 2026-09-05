//go:build unix

package process

import (
	"errors"
	"os"
	"os/exec"
	"syscall"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
)

func buildCommand(service domain.ServiceSpec) *exec.Cmd {
	cmd := exec.Command("sh", "-c", service.Command)
	cmd.Dir = service.Directory
	cmd.Env = mergeEnv(os.Environ(), service.Env)

	// New process group: isolates the child from the daemon's own
	// terminal signal delivery (SIGINT/SIGTERM to the daemon's foreground
	// group must not reach services it manages — Stop() is the only
	// sanctioned termination path).
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}

	cmd.Stdout = nil
	cmd.Stderr = nil
	cmd.Stdin = nil

	return cmd
}

// terminateGraceful signals the whole process group, not just the direct
// child. service.Command runs through "sh -c", so the direct child is the
// shell — signalling only its PID leaves the real payload process running
// as an orphan after the shell exits.
func terminateGraceful(proc *os.Process) error {
	return syscall.Kill(-proc.Pid, syscall.SIGTERM)
}

func killForceful(proc *os.Process) error {
	return syscall.Kill(-proc.Pid, syscall.SIGKILL)
}

func isProcessDone(err error) bool {
	if err == nil {
		return false
	}
	if errors.Is(err, os.ErrProcessDone) {
		return true
	}
	var errno syscall.Errno
	if errors.As(err, &errno) {
		// ESRCH: No such process (Linux/POSIX)
		// EPERM: Operation not permitted
		return errors.Is(errno, syscall.ESRCH) || errors.Is(errno, syscall.EPERM)
	}
	return false
}
