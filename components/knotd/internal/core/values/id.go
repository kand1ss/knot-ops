package values

import "github.com/google/uuid"

type WorkspaceId uuid.UUID

func NewWorkspaceId() WorkspaceId {
	return WorkspaceId(uuid.New())
}

type ExecutionID uuid.UUID

func NewExecutionId() ExecutionID {
	return ExecutionID(uuid.New())
}

func (s ExecutionID) String() string {
	return uuid.UUID(s).String()
}

type TaskId uuid.UUID

func NewTaskId() TaskId {
	return TaskId(uuid.New())
}

func (t TaskId) String() string {
	return uuid.UUID(t).String()
}
