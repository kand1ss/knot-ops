package values

import "github.com/google/uuid"

type WorkspaceId uuid.UUID

func NewWorkspaceId() WorkspaceId {
	return WorkspaceId(uuid.New())
}
