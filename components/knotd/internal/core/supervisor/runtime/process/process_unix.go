//go:build unix

package process

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"syscall"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	shellquote "github.com/kballard/go-shellquote"
)

// buildCommand prepares an *exec.Cmd to run a service on Unix systems.
//
// It parses service.Command via shellquote.Split to execute the binary directly,
// bypassing `sh -c` to ensure exact PID tracking, eliminate shell-wrapper orphan
// processes, and prevent shell injection risks.
//
// It also sets SysProcAttr.Setpgid = true to isolate the service in its own process
// group, protecting it from terminal signals (SIGINT/SIGTERM) sent to the parent daemon.
func buildCommand(service domain.ServiceSpec) (*exec.Cmd, error) {
	if strings.TrimSpace(service.Command) == "" {
		return nil, fmt.Errorf("command cannot be empty or whitespace-only")
	}

	args, err := shellquote.Split(service.Command)
	if err != nil {
		return nil, fmt.Errorf("invalid command line: %w", err)
	}

	if len(args) == 0 {
		return nil, fmt.Errorf("command line resolved to empty arguments")
	}

	cmd := exec.Command(args[0], args[1:]...)
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

	return cmd, nil
}

// attachToContainer is a no-op on unix: process-group signaling (see
// terminateGraceful/killForceful below) already covers the whole tree
// spawned under the shell, so there's no extra containment to set up.
func attachToContainer(_ *exec.Cmd) (processContainer, error) {
	return nil, nil
}

func resumeProcess(_ int) error {
	return nil
}

// releaseContainer is a no-op on unix — there's no container-side handle
// to release; the process group itself needs no explicit cleanup.
func releaseContainer(_ processContainer) {}

// terminateGraceful signals the whole process group, not just the direct
// child. Even with buildCommand's "exec" guaranteeing sh and the payload
// share one PID, a payload that itself spawns children into this group
// (rather than merely being sh-in-place-of) can still leave stragglers
// behind — group-signal is defense in depth, not a workaround for the
// PID-identity issue (that's handled in buildCommand now).
func terminateGraceful(proc *os.Process, _ processContainer) error {
	return syscall.Kill(-proc.Pid, syscall.SIGTERM)
}

func killForceful(proc *os.Process, _ processContainer) error {
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
