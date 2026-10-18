package values

import "github.com/google/uuid"

type WorkspaceId uuid.UUID

func NewWorkspaceId() WorkspaceId {
	return WorkspaceId(uuid.New())
}

type TaskID uuid.UUID

func NewTaskID() TaskID {
	return TaskID(uuid.New())
}

func (s TaskID) String() string {
	return uuid.UUID(s).String()
}
