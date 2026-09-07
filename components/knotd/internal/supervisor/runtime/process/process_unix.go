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
	// "exec" forces sh to replace its own process image (execve) with the
	// payload instead of relying on shell-implementation-defined behavior
	// for whether a single simple command gets exec'd in place or forked.
	// Without this, cmd.Process.Pid (and therefore cmd.Wait()'s reaping)
	// tracks sh itself on shells/versions that fork rather than exec — so a
	// group-wide SIGTERM kills the (unprotected) shell instantly, Wait()
	// reports "exited", and the real payload silently survives as an
	// untracked orphan. exec collapses shell and payload into one PID,
	// making that ambiguity impossible.
	//
	// Trade-off: only the final exec'd command in service.Command actually
	// runs — "exec a && b" never reaches b, since the process image is
	// replaced before "&&" is evaluated. Fine for a single command + args;
	// do not use compound shell scripts in ServiceSpec.Command.
	cmd := exec.Command("sh", "-c", "exec "+service.Command)
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
// child. Even with buildCommand's "exec" guaranteeing sh and the payload
// share one PID, a payload that itself spawns children into this group
// (rather than merely being sh-in-place-of) can still leave stragglers
// behind — group-signal is defense in depth, not a workaround for the
// PID-identity issue (that's handled in buildCommand now).
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
