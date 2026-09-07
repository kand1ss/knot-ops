//go:build windows

package process

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"syscall"
	"unsafe"

	"golang.org/x/sys/windows"

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

// windowsJob wraps the Job Object a service's process tree is assigned to.
// TerminateJobObject kills every process ever assigned to it in one call,
// regardless of how deep "cmd /C" spawned the real payload — this is what
// closes the wrapper-vs-payload identity gap that made proc.Kill() only
// ever kill cmd.exe and orphan the actual service.
type windowsJob struct {
	handle windows.Handle
}

// attachToContainer creates a Job Object, sets JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
// (so a crashed/killed knotd doesn't leak orphaned service trees even without
// an explicit Stop()), and assigns cmd's process to it.
//
// os.Process does not expose the process handle CreateProcess originally
// returned, so we reopen one by PID with just enough access rights to do
// the assignment.
//
// Known residual race: any child cmd.exe spawns *before* this function
// completes will not be a job member (job membership is not retroactive).
// Closing it fully would require creating cmd.exe CREATE_SUSPENDED and
// resuming it only after assignment, which needs the CreateProcess thread
// handle — something os/exec does not expose. In practice cmd.exe spends
// measurable time on its own startup before it spawns the payload, so the
// window is small; it is not eliminated, and that's stated here rather
// than silently assumed away.
func attachToContainer(cmd *exec.Cmd) (processContainer, error) {
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return nil, fmt.Errorf("CreateJobObject: %w", err)
	}

	info := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{
		BasicLimitInformation: windows.JOBOBJECT_BASIC_LIMIT_INFORMATION{
			LimitFlags: windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
		},
	}
	if _, err := windows.SetInformationJobObject(
		job,
		windows.JobObjectExtendedLimitInformation,
		uintptr(unsafe.Pointer(&info)),
		uint32(unsafe.Sizeof(info)),
	); err != nil {
		_ = windows.CloseHandle(job)
		return nil, fmt.Errorf("SetInformationJobObject: %w", err)
	}

	procHandle, err := windows.OpenProcess(
		windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE,
		false,
		uint32(cmd.Process.Pid),
	)
	if err != nil {
		_ = windows.CloseHandle(job)
		return nil, fmt.Errorf("OpenProcess pid %d: %w", cmd.Process.Pid, err)
	}
	defer func() { _ = windows.CloseHandle(procHandle) }()

	if err := windows.AssignProcessToJobObject(job, procHandle); err != nil {
		_ = windows.CloseHandle(job)
		return nil, fmt.Errorf("AssignProcessToJobObject pid %d: %w", cmd.Process.Pid, err)
	}

	return &windowsJob{handle: job}, nil
}

// releaseContainer closes our handle to the Job Object once the tracked
// process has exited. This does not itself kill anything: closing the last
// handle to a job normally just frees kernel resources. It only becomes a
// kill if JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE is set (as attachToContainer
// does), which is the deliberate safety net for a knotd crash mid-lifecycle.
func releaseContainer(container processContainer) {
	job, ok := container.(*windowsJob)
	if !ok || job == nil {
		return
	}
	_ = windows.CloseHandle(job.handle)
}

// terminateGraceful: Windows has no SIGTERM reachable from a Go process
// belonging to a different console session, and GenerateConsoleCtrlEvent
// against a "cmd /C"-spawned tree is unreliable in practice. This is
// force-kill-only on Windows until a proper CTRL_BREAK_EVENT path is
// built — stated honestly rather than faked as graceful. It now at least
// force-kills the *whole tree* via the job, instead of only cmd.exe.
func terminateGraceful(proc *os.Process, container processContainer) error {
	return killForceful(proc, container)
}

func killForceful(proc *os.Process, container processContainer) error {
	if job, ok := container.(*windowsJob); ok && job != nil {
		return windows.TerminateJobObject(job.handle, 1)
	}
	// No job available — attachToContainer failed or wasn't wired up for
	// this instance. Fall back to the old, incomplete behavior rather than
	// doing nothing; Start() is expected to have already rejected this case
	// (see runtime.go), so reaching here indicates that guard was bypassed.
	return proc.Kill()
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
		return errors.Is(errno, syscall.ERROR_ACCESS_DENIED)
	}
	return false
}
