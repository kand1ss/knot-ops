package task

type TaskGroup struct {
	Header string
	Tasks  []TaskPlan
}

type TaskPlan struct {
	TaskId string
	Name   string
	Detail string
}
