//! Public error/success types that may be returned by functions of this crate

pub(crate) struct TaskResult<E> {
	pub(crate) name: String,
	pub(crate) result: Result<(), TaskErrorKind<E>>,
}

/// When handling tasks errors individually through
/// [`TasksMainHandle::join_all_with`](crate::TasksMainHandle::join_all_with), you get this, giving details about what
/// task it was and why exactly it didn't succeed
#[derive(thiserror::Error)]
#[error("Task {task_name} errored: {kind}")]
pub struct TaskError<E> {
	pub(crate) task_name: String,
	pub(crate) kind: TaskErrorKind<E>,
}

impl<E> TaskError<E> {
	pub fn task_name(&self) -> &str {
		&self.task_name
	}

	pub fn kind(&self) -> &TaskErrorKind<E> {
		&self.kind
	}

	pub fn into_kind(self) -> TaskErrorKind<E> {
		self.kind
	}
}

/// Contained inside a [`TaskError`]
///
/// When handling tasks errors individually through
/// [`TasksMainHandle::join_all_with`](crate::TasksMainHandle::join_all_with), you get this, giving details about why
/// exactly it didn't succeed
#[derive(thiserror::Error)]
pub enum TaskErrorKind<E> {
	#[error("User error: {0}")]
	UserError(E),
	#[error("Tokio join error: {0}")]
	TokioJoinError(tokio::task::JoinError),
	#[error("Cancel timeout exceeded - left task dangle")]
	CancelTimeoutExceeded(tokio::task::JoinHandle<Result<(), E>>),
}

/// This error is returned from the [`TasksMainHandle::join_all`](crate::TasksMainHandle::join_all) methods
/// if at least one of the tasks has errored
#[derive(thiserror::Error, Debug)]
#[error("At least one task errored")]
pub struct AtLeastOneTaskErrored {
	pub(crate) _private: (),
}

/// This error is returned from a [`spawn`](crate::TasksHandle::spawn) if
/// we are already shutting down, and consequently can't spawn the task
#[derive(thiserror::Error, Debug)]
#[error("Task {task_name} was not spawned because we were already stopping")]
pub struct TasksAreStopping<TaskType> {
	pub(crate) task_name: String,
	pub(crate) task_that_failed_to_start: TaskType,
}

impl<TaskType> TasksAreStopping<TaskType> {
	pub fn task_name(&self) -> &str {
		&self.task_name
	}

	pub fn into_task_that_failed_to_start(self) -> TaskType {
		self.task_that_failed_to_start
	}
}
