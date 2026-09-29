package task

type ExecutionPlan struct {
	ExecutionID string
	Groups      []TaskGroup
}

func (c ExecutionPlan) IsSyncEvent() {}
func (c ExecutionPlan) IsUpEvent()   {}
func (c ExecutionPlan) IsDownEvent() {}

type TaskError struct {
	Issue    string
	Context  string
	Solution string
}

type TaskStarting struct {
	TaskID string
	Detail string
}

func (t TaskStarting) IsSyncEvent() {}
func (t TaskStarting) IsUpEvent()   {}
func (t TaskStarting) IsDownEvent() {}

type TaskSuccess struct {
	TaskID string
	Detail string
}

func (t TaskSuccess) IsSyncEvent() {}
func (t TaskSuccess) IsUpEvent()   {}
func (t TaskSuccess) IsDownEvent() {}

type TaskFailed struct {
	TaskID string
	Error  TaskError
}

func (t TaskFailed) IsSyncEvent() {}
func (t TaskFailed) IsUpEvent()   {}
func (t TaskFailed) IsDownEvent() {}

type TaskSkipped struct {
	TaskID string
	Detail string
}

func (t TaskSkipped) IsSyncEvent() {}
func (t TaskSkipped) IsUpEvent()   {}
func (t TaskSkipped) IsDownEvent() {}

type TaskCancelled struct {
	TaskID string
	Detail string
}

func (t TaskCancelled) IsSyncEvent() {}
func (t TaskCancelled) IsUpEvent()   {}
func (t TaskCancelled) IsDownEvent() {}
