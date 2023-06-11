mod named_task;
mod results;

use {named_task::NamedTask, results::*};

use {
	futures::{prelude::*, stream::futures_unordered::FuturesUnordered},
	log::{debug, trace, warn},
	std::{fmt, sync::Arc},
	tokio::{signal, sync::mpsc, task::JoinHandle},
	tokio_util::sync::CancellationToken,
};

pub struct SystemsMaster<E> {
	results: mpsc::UnboundedReceiver<SystemError<E>>,
	handle: SystemsHandle<E>,
	systems_management_task: JoinHandle<()>,
}

#[derive(Debug, Default)]
pub struct SystemsMasterBuilder {
	timeout: Option<std::time::Duration>,
	dont_catch_signals: bool,
}

pub struct SystemsHandle<E> {
	systems: Arc<Systems<E>>,
}

impl<E> Clone for SystemsHandle<E> {
	fn clone(&self) -> Self {
		Self {
			systems: self.systems.clone(),
		}
	}
}

struct Systems<E> {
	tasks_sender: arc_swap::ArcSwapOption<mpsc::UnboundedSender<NamedTask<E>>>,
	should_stop: CancellationToken,
}

impl SystemsMasterBuilder {
	pub fn build<E: Send + fmt::Display + 'static>(self) -> SystemsMaster<E> {
		let (tasks_sender, mut tasks_receiver) = mpsc::unbounded_channel::<NamedTask<E>>();
		let (results_sender, results_receiver) = mpsc::unbounded_channel::<SystemError<E>>();
		let should_stop = CancellationToken::new();

		let systems_handle = SystemsHandle {
			systems: Arc::new(Systems {
				tasks_sender: arc_swap::ArcSwapOption::new(Some(Arc::new(tasks_sender))),
				should_stop,
			}),
		};

		let systems_handle_clone = systems_handle.clone();
		let catch_signals = !self.dont_catch_signals;

		let systems_management_task = tokio::task::spawn(async move {
			let mut all_systems = FuturesUnordered::new();
			let shutdown_timeout = async {
				if let Some(timeout) = self.timeout {
					systems_handle.systems.should_stop.cancelled().await;
					tokio::time::sleep(timeout).await
				} else {
					future::pending().await
				}
			};
			tokio::pin!(shutdown_timeout);
			let mut aborting = false;
			let mut tasks_receiver_has_shut_down = false;
			loop {
				tokio::select! {
					biased;
					_ = signal::ctrl_c(), if catch_signals => {
						systems_handle.start_shutdown();
					}
					_ = &mut shutdown_timeout, if !aborting && self.timeout.is_some() => {
						warn!("Graceful stopping timeout reached - aborting tasks");
						aborting = true;
						all_systems.iter_mut().for_each(|f: &mut NamedTask<_>| {
							trace!("Aborting task {}", f.name());
							f.abort()
						});
					}
					new_task_to_listen_for = tasks_receiver.recv(), if !tasks_receiver_has_shut_down => {
						match new_task_to_listen_for {
							None => {
								tasks_receiver_has_shut_down = true;
								if !all_systems.is_empty() {
									trace!("All SystemsHandle have been dropped - cancelling everything and exiting");
									systems_handle.start_shutdown();
								} else {
									trace!("Task channel closed - exiting system management task");
									break;
								}
							}
							Some(new_task) => {
								trace!("Registering task: {}", new_task.name());
								if aborting {
									trace!("We are already stopping, so {} will be aborted right away", new_task.name());
									new_task.abort();
								}
								all_systems.push(new_task);
							}
						}
					}
					task_finished = all_systems.next(), if !all_systems.is_empty() => {
						let res: TaskResult<E> = task_finished.expect("Branch is disabled so we should never get None");
						trace!("Got result for system {}", res.name);
						if let Err(kind) = res.result {
							debug!("System {} errored: {kind}, starting shutdown...", res.name);
							systems_handle.systems.should_stop.cancel();
							// If user doesn't care about results it's fine
							let _: Result<_, mpsc::error::SendError<_>> = results_sender
								.send(SystemError{ system_name: res.name, kind });
						}
						if tasks_receiver_has_shut_down && all_systems.is_empty() {
							trace!("Received last result - exiting system management task");
							break;
						}
					}
				};
			}
		});

		SystemsMaster {
			results: results_receiver,
			handle: systems_handle_clone,
			systems_management_task,
		}
	}
}

impl<E: Send + 'static> SystemsMaster<E> {
	/// Create a new handle to this master
	///
	/// Note that the master has Deref on the Handle, so if you already have the master
	/// at hand, you don't need to spawn a handle.
	pub fn handle(&self) -> SystemsHandle<E> {
		self.handle.clone()
	}

	/// Wait for the systems to close, doing something with the errors as they are encountered
	pub async fn with_errors(self, mut f: impl FnMut(SystemError<E>)) -> Result<(), AtLeastOneSystemErrored> {
		self.with_errors_async(|e| {
			f(e);
			future::ready(())
		})
		.await
	}

	/// Wait for the systems to close, doing something with the errors as they are encountered
	pub async fn with_errors_async<F, Fut>(mut self, mut f: F) -> Result<(), AtLeastOneSystemErrored>
	where
		F: FnMut(SystemError<E>) -> Fut,
		Fut: Future<Output = ()>,
	{
		let mut res = Ok(());
		while let Some(e) = self.results.recv().await {
			res = Err(AtLeastOneSystemErrored { _private: () });
			f(e).await;
		}
		self.systems_management_task
			.await
			.expect("Systems management task did not close successfully");
		res
	}
}

impl<E> std::ops::Deref for SystemsMaster<E> {
	type Target = SystemsHandle<E>;

	fn deref(&self) -> &Self::Target {
		&self.handle
	}
}

impl<E: Send + fmt::Debug + 'static> SystemsHandle<E> {
	pub fn spawn<F, Fut>(&self, system_name: impl Into<String>, f: F) -> Result<(), SystemsAreStopping<()>>
	where
		F: FnOnce(SystemsHandle<E>) -> Fut,
		Fut: Future<Output = Result<(), E>> + Send + 'static,
	{
		self.spawn_advanced(system_name, (), |()| tokio::task::spawn(f(self.clone())))
	}

	pub fn spawn_advanced<SystemType, SpawnFn>(
		&self,
		system_name: impl Into<String>,
		system_type: SystemType,
		spawn: SpawnFn,
	) -> Result<(), SystemsAreStopping<SystemType>>
	where
		SpawnFn: FnOnce(SystemType) -> JoinHandle<Result<(), E>>,
	{
		let name = system_name.into();
		let tasks_sender_guard = self.systems.tasks_sender.load();
		if let Some(tasks_sender) = &*tasks_sender_guard {
			trace!("Spawning task {name}");
			let task = spawn(system_type);
			tasks_sender
				.send(NamedTask { name: Some(name), task })
				.expect("Receiving end of the tasks shouldn't have stopped by itself");
			Ok(())
		} else {
			// If user doesn't care about results it's fine
			trace!("Not spawning task {name} because already stopping");
			Err(SystemsAreStopping {
				system_name: name,
				system_that_failed_to_start: system_type,
				_private: (),
			})
		}
	}
}

impl<E> SystemsHandle<E> {
	pub fn start_shutdown(&self) {
		debug!("Starting graceful shutdown");
		self.systems.should_stop.cancel();
		self.systems.tasks_sender.store(None);
	}

	pub fn on_shutdown(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
		self.systems.should_stop.cancelled()
	}
}

impl SystemsMasterBuilder {
	pub fn timeout(mut self, timeout: std::time::Duration) -> Self {
		self.timeout = Some(timeout);
		self
	}

	pub fn dont_catch_signals(mut self) -> Self {
		self.dont_catch_signals = true;
		self
	}
}
