package task

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

type TaskEvent interface {
	IsTaskEvent()
}

type TaskAccepted struct {
	TaskID   values.TaskID
	Services []values.ServiceName
}

func (c TaskAccepted) IsTaskEvent() {}

type TaskCancelled struct {
	TaskID  values.TaskID
	Message string
}

func (t TaskCancelled) IsTaskEvent() {}

type TaskDone struct {
	TaskID  values.TaskID
	Started uint32
	Stopped uint32
	Skipped uint32
	Failed  uint32
}

func (t TaskDone) IsTaskEvent() {}

type CancelTaskCode int

const (
	CancelTaskCodeSuccess = iota
	CancelTaskCodeNotFound
	CancelTaskCodeAlreadyCompleted
	CancelTaskCodeAlreadyCancelling
)

type CancelTaskRequest struct {
	TaskID values.TaskID
}

type CancelTaskResponse struct {
	Cancelled bool
	Code      CancelTaskCode
}
