package values

import "github.com/google/uuid"

type WorkspaceID uuid.UUID

func NewWorkspaceId() WorkspaceID {
	return WorkspaceID(uuid.New())
}

func (s WorkspaceID) String() string {
	return uuid.UUID(s).String()
}

type TaskID uuid.UUID

func NewTaskID() TaskID {
	return TaskID(uuid.New())
}

func (s TaskID) String() string {
	return uuid.UUID(s).String()
}
