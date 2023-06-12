//! Easily manage and gracefully shutdown tokio tasks while monitoring their return results
//!
//! # Example
//! ```
//! use {std::time::Duration, tokio::time::sleep, tokio_tasks_shutdown::*};
//!
//! # tokio_test::block_on(async {
//! # let start = std::time::Instant::now();
//! // By default this will catch Ctrl+C.
//! // You may have your tasks return your own error type.
//! let tasks: TasksMainHandle<anyhow::Error> = TasksBuilder::default()
//! 	.timeouts(Some(Duration::from_secs(2)), Some(Duration::from_millis(500)))
//! 	.build();
//!
//! // Let's simulate a Ctrl+C after some time
//! let tasks_handle: TasksHandle<_> = tasks.handle();
//! tokio::task::spawn(async move {
//! 	sleep(Duration::from_millis(150)).await;
//! 	tasks_handle.start_shutdown();
//! });
//!
//! // Spawn tasks
//! tasks
//! 	.spawn("gracefully_shutting_down_task", |tasks_handle| async move {
//! 		loop {
//! 			tokio::select! {
//! 				biased;
//! 				_ = tasks_handle.on_shutdown() => {
//! 					// We have been kindly asked to shutdown, let's exit
//! 					break;
//! 				}
//! 				_ = sleep(Duration::from_millis(100)) => {
//! 					// Simulating another task running concurrently, e.g. listening on a channel...
//! 				}
//! 			}
//! 		}
//! 		Ok(())
//! 		// Note that if a task were to error, graceful shutdown would be initiated.
//! 		// This behavior can be disabled.
//! 	})
//! 	.unwrap();
//! // Note that calls can be chained since `spawn` returns `&TasksHandle`
//!
//! // Let's make sure there were no errors
//! tasks.join_all().await.unwrap();
//!
//! # let test_duration = start.elapsed();
//! assert!(
//! 	test_duration > Duration::from_millis(145) && test_duration < Duration::from_millis(155)
//! );
//! # })
//! ```
//!
//! In this example, the task will have run one loop already (sleep has hit at t=100ms) when asked for graceful
//! shutdown at t=150ms, which will immediately make it gracefully shut down.

mod named_task;
pub mod results;

use results::*;

use named_task::NamedTask;

use {
	futures::{prelude::*, stream::futures_unordered::FuturesUnordered},
	log::{debug, error, trace, warn},
	std::{fmt, sync::Arc},
	tokio::{signal, sync::mpsc, task::JoinHandle},
	tokio_util::sync::CancellationToken,
};

/// Main handle to the set of tasks. This is the only one that may be used to collect their results.
///
/// Note that shut down will be automatically initiated if this is dropped.
pub struct TasksMainHandle<E> {
	results_receiver: mpsc::UnboundedReceiver<TaskError<E>>,
	handle: TasksHandle<E>,
	management_task: Option<JoinHandle<()>>,
}

/// Builder for the set of tasks [`TasksMainHandle`]
#[derive(Debug)]
pub struct TasksBuilder {
	graceful_shutdown_timeout: Option<std::time::Duration>,
	task_abort_timeout: Option<std::time::Duration>,
	catch_signals: bool,
	shutdown_if_a_task_errors: bool,
}

/// Handle to a set of tasks.
/// Can be used to spawn new tasks, initiate a shutdown or check shutdown status.
///
/// Access to spawning and shutting down, but not to getting results of finished tasks:
/// only the [`TasksMainHandle`] has access to that.
pub struct TasksHandle<E> {
	inner: Arc<TasksInner<E>>,
}

impl<E> Clone for TasksHandle<E> {
	fn clone(&self) -> Self {
		Self {
			inner: self.inner.clone(),
		}
	}
}

struct TasksInner<E> {
	tasks_sender: arc_swap::ArcSwapOption<mpsc::UnboundedSender<NamedTask<E>>>,
	should_stop: CancellationToken,
}

impl TasksBuilder {
	/// Build the [`TasksMainHandle`], that can then be used to spawn tasks and obtain their results
	///
	/// [`Display`](fmt::Display) is required on `E` because an error would be printed out to [`log`].
	pub fn build<E: Send + fmt::Display + 'static>(self) -> TasksMainHandle<E> {
		let (tasks_sender, mut tasks_receiver) = mpsc::unbounded_channel::<NamedTask<E>>();
		let (results_sender, results_receiver) = mpsc::unbounded_channel::<TaskError<E>>();
		let should_stop = CancellationToken::new();

		let tasks_handle = TasksHandle {
			inner: Arc::new(TasksInner {
				tasks_sender: arc_swap::ArcSwapOption::new(Some(Arc::new(tasks_sender))),
				should_stop,
			}),
		};

		let tasks_handle_clone = tasks_handle.clone();

		let management_task = tokio::task::spawn(async move {
			let mut all_tasks = FuturesUnordered::new();
			let shutdown_timeout = async {
				if let Some(timeout) = self.graceful_shutdown_timeout {
					tasks_handle.inner.should_stop.cancelled().await;
					tokio::time::sleep(timeout).await
				} else {
					future::pending().await
				}
			};
			tokio::pin!(shutdown_timeout);
			let exit_letting_tasks_dangle_timeout = async {
				// The timeout will only start the first time this is polled, after the `aborting`
				// boolean has been set
				if let Some(timeout) = self.task_abort_timeout {
					tasks_handle.inner.should_stop.cancelled().await;
					tokio::time::sleep(timeout).await
				} else {
					future::pending().await
				}
			};
			tokio::pin!(exit_letting_tasks_dangle_timeout);
			let mut aborting = false;
			let mut tasks_receiver_has_shut_down = false;
			loop {
				tokio::select! {
					biased;
					_ = signal::ctrl_c(), if self.catch_signals => {
						tasks_handle.start_shutdown();
					}
					_ = &mut shutdown_timeout, if !aborting && self.graceful_shutdown_timeout.is_some() => {
						warn!("Graceful stopping timeout reached - aborting tasks");
						aborting = true;
						all_tasks.iter_mut().for_each(|f: &mut NamedTask<_>| {
							trace!("Aborting task {}", f.name());
							f.abort()
						});
					}
					new_task_to_listen_for = tasks_receiver.recv(), if !tasks_receiver_has_shut_down => {
						match new_task_to_listen_for {
							None => {
								tasks_receiver_has_shut_down = true;
								if all_tasks.is_empty() {
									trace!("Task channel closed - exiting management task");
									break;
								} else {
									debug_assert!(tasks_handle.is_shutting_down());
								}
							}
							Some(new_task) => {
								trace!("Registering task: {}", new_task.name());
								if aborting {
									trace!("We are already stopping, so {} will be aborted right away", new_task.name());
									new_task.abort();
								}
								all_tasks.push(new_task);
							}
						}
					}
					task_finished = all_tasks.next(), if !all_tasks.is_empty() => {
						let res: TaskResult<E> = task_finished.expect("Branch is disabled so we should never get None");
						trace!("Got result for task {}", res.name);
						if let Err(kind) = res.result {
							let shutting_down = self.shutdown_if_a_task_errors && !tasks_handle.is_shutting_down();
							error!(
								"Task {} errored: {kind}{}",
								res.name,
								if shutting_down {", starting shutdown..."} else {""}
							);
							if shutting_down {
								tasks_handle.start_shutdown();
							}
							// If user doesn't care about results it's fine
							let _: Result<_, mpsc::error::SendError<_>> = results_sender
								.send(TaskError{ task_name: res.name, kind });
						}
						if tasks_receiver_has_shut_down && all_tasks.is_empty() {
							trace!("Received last result - exiting management task");
							break;
						}
					}
					_ = &mut exit_letting_tasks_dangle_timeout, if aborting && tasks_receiver_has_shut_down && self.task_abort_timeout.is_some() => {
						error!("Abort timeout reached - letting tasks dangle and exiting management task");
						for task in std::mem::take(&mut all_tasks) {
							let task_name = task.name.expect("Hasn't resolved");
							trace!("Sending dangling task error for {task_name}");
							let _: Result<_, mpsc::error::SendError<_>> = results_sender
								.send(TaskError{
									task_name: task_name,
									kind: TaskErrorKind::CancelTimeoutExceeded(task.task)
								});
						}
						break;
					}
				};
			}
		});

		TasksMainHandle {
			results_receiver,
			handle: tasks_handle_clone,
			management_task: Some(management_task),
		}
	}
}

impl<E> Drop for TasksMainHandle<E> {
	fn drop(&mut self) {
		self.start_shutdown();
	}
}

impl<E: Send + 'static> TasksMainHandle<E> {
	/// Create a new handle to this set of tasks
	///
	/// Note that `TasksMainHandle` has [`Deref`](std::ops::Deref) on the [`TasksHandle`], so if you already have the
	/// `TasksMainHandle` at hand, you don't need to create a new handle to e.g. [`spawn`](TasksHandle::spawn) a task.
	pub fn handle(&self) -> TasksHandle<E> {
		self.handle.clone()
	}

	/// Wait for the tasks to finish
	///
	/// Tasks that have errored will be displayed in [`log`] at the `Error` level.
	///
	/// If you need to get at least one of those, use [`join_all_with`](TasksMainHandle::join_all_with) or
	/// [`join_all_yielding_on_first_error`](TasksMainHandle::join_all_yielding_on_first_error)
	pub async fn join_all(self) -> Result<(), AtLeastOneTaskErrored> {
		self.join_all_with(|_| {}).await
	}

	/// Wait for the tasks to finish
	///
	/// As soon as at least one error is returned, this will yield, letting the rest of the tasks
	/// close later
	pub async fn join_all_yielding_on_first_error(&mut self) -> Result<(), TaskError<E>> {
		if let Some(e) = self.results_receiver.recv().await {
			Err(e)
		} else {
			self.ensure_management_task_closes_properly().await;
			Ok(())
		}
	}

	/// Wait for the tasks to finish, doing something with the errors as they are encountered
	pub async fn join_all_with(mut self, mut f: impl FnMut(TaskError<E>)) -> Result<(), AtLeastOneTaskErrored> {
		let mut res = Ok(());
		while let Some(e) = self.results_receiver.recv().await {
			res = Err(AtLeastOneTaskErrored { _private: () });
			f(e);
		}
		self.ensure_management_task_closes_properly().await;
		res
	}

	/// Wait for the tasks to finish, doing something with the errors as they are encountered
	pub async fn join_all_with_async<F, Fut>(mut self, mut f: F) -> Result<(), AtLeastOneTaskErrored>
	where
		F: FnMut(TaskError<E>) -> Fut,
		Fut: Future<Output = ()>,
	{
		let mut res = Ok(());
		while let Some(e) = self.results_receiver.recv().await {
			res = Err(AtLeastOneTaskErrored { _private: () });
			f(e).await;
		}
		self.ensure_management_task_closes_properly().await;
		res
	}

	async fn ensure_management_task_closes_properly(&mut self) {
		if let Some(management_task) = self.management_task.take() {
			management_task
				.await
				.expect("Task management task did not close successfully");
		}
	}
}

impl<E> std::ops::Deref for TasksMainHandle<E> {
	type Target = TasksHandle<E>;

	fn deref(&self) -> &Self::Target {
		&self.handle
	}
}

impl<E: Send + fmt::Debug + 'static> TasksHandle<E> {
	/// Spawn a future on the tokio runtime if the Tasks aren't already stopping
	///
	/// The future should be built from the provided [`TasksHandle`], and most likely monitor graceful shutdown status.
	pub fn spawn<F, Fut>(&self, task_name: impl Into<String>, f: F) -> Result<&Self, TasksAreStopping<()>>
	where
		F: FnOnce(TasksHandle<E>) -> Fut,
		Fut: Future<Output = Result<(), E>> + Send + 'static,
	{
		self.spawn_advanced(task_name, (), |()| tokio::task::spawn(f(self.clone())))
	}

	/// Spawn a blocking task on the tokio runtime if the Tasks aren't already stopping
	pub fn spawn_blocking<F>(&self, task_name: impl Into<String>, f: F) -> Result<&Self, TasksAreStopping<()>>
	where
		F: FnOnce(TasksHandle<E>) -> Result<(), E> + Send + 'static,
	{
		self.spawn_advanced(task_name, (), |()| {
			let handle = self.clone();
			tokio::task::spawn_blocking(move || f(handle))
		})
	}

	/// A more flexible version of `spawn`
	pub fn spawn_advanced<TaskType, SpawnFn>(
		&self,
		task_name: impl Into<String>,
		task_type: TaskType,
		spawn: SpawnFn,
	) -> Result<&Self, TasksAreStopping<TaskType>>
	where
		SpawnFn: FnOnce(TaskType) -> JoinHandle<Result<(), E>>,
	{
		let name = task_name.into();
		let tasks_sender_guard = self.inner.tasks_sender.load();
		if let Some(tasks_sender) = &*tasks_sender_guard {
			trace!("Spawning task {name}");
			let task = spawn(task_type);
			tasks_sender
				.send(NamedTask { name: Some(name), task })
				.expect("Receiving end of the tasks shouldn't have stopped by itself");
			Ok(self)
		} else {
			// If user doesn't care about results it's fine
			trace!("Not spawning task {name} because already stopping");
			Err(TasksAreStopping {
				task_name: name,
				task_that_failed_to_start: task_type,
			})
		}
	}
}

impl<E> TasksHandle<E> {
	pub fn start_shutdown(&self) {
		debug!("Starting graceful shutdown");
		self.inner.should_stop.cancel();
		self.inner.tasks_sender.store(None);
	}

	/// This future will resolve when graceful shutdown was asked
	pub fn on_shutdown(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
		self.inner.should_stop.cancelled()
	}

	/// Whether graceful shutdown was asked
	pub fn is_shutting_down(&self) -> bool {
		self.inner.should_stop.is_cancelled()
	}
}

impl Default for TasksBuilder {
	fn default() -> Self {
		Self {
			graceful_shutdown_timeout: None,
			task_abort_timeout: None,
			catch_signals: true,
			shutdown_if_a_task_errors: true,
		}
	}
}

impl TasksBuilder {
	/// Set timeouts for graceful shutdown and tokio task abort
	///
	/// If timeout is exceeded after asking for graceful shutdown, tokio tasks will be
	/// [`abort`](tokio::task::JoinHandle::abort)ed.
	///
	/// If that doesn't make them yield after an extra `task_abort_timeout` and you are running on the multi-threaded
	/// `tokio` runtime, they will *typically* be left dangling, and the [`join_all`](TasksMainHandle::join_all)
	/// function will still return. This is
	/// [typically](https://github.com/tokio-rs/tokio/issues/4730#issuecomment-1147165954)
	/// not 100% reliable because if `tokio` has assigned your freezing-not-yielding
	/// task to the same thread as the task that monitors this timeout, it may still freeze.
	///
	/// To make it 100% reliable, use:
	/// ```rust
	/// # tokio_test::block_on(async {
	/// // Apply the workaround described at https://github.com/tokio-rs/tokio/issues/4730#issuecomment-1147165954
	/// // to make `task_abort_timeout` 100% reliable
	/// let rt_handle = tokio::runtime::Handle::current();
	/// std::thread::spawn(move || loop {
	/// 	std::thread::sleep(std::time::Duration::from_millis(500));
	/// 	rt_handle.spawn(std::future::ready(()));
	/// });
	/// # })
	/// ```
	///
	/// (If you are not on a multi-threaded tokio runtime, a freezing task that would never yield would always prevent
	/// this `task_abort_timeout` from executing)
	///
	/// (See [`tokio#4730`](https://github.com/tokio-rs/tokio/issues/4730) for more details)
	pub fn timeouts(
		mut self,
		graceful_shutdown_timeout: Option<std::time::Duration>,
		task_abort_timeout: Option<std::time::Duration>,
	) -> Self {
		self.graceful_shutdown_timeout = graceful_shutdown_timeout;
		self.task_abort_timeout = task_abort_timeout;
		self
	}

	/// Disable graceful shutdown on Ctrl+C
	///
	/// By default, Ctrl+C will initiate a shutdown
	pub fn dont_catch_signals(mut self) -> Self {
		self.catch_signals = false;
		self
	}

	/// Disable graceful shutdown when one task returns an error
	///
	/// By default, if any task returns `Err(_)`, the system will gracefully shutdown immediately.
	pub fn dont_shutdown_if_a_task_errors(mut self) -> Self {
		self.shutdown_if_a_task_errors = false;
		self
	}
}
