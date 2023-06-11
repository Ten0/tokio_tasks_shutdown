mod named_task;
mod results;

use {named_task::NamedTask, results::*};

use {
	futures::{prelude::*, stream::futures_unordered::FuturesUnordered},
	log::{debug, error, trace},
	std::{fmt, sync::Arc},
	tokio::{signal, sync::mpsc, task::JoinHandle},
	tokio_util::sync::CancellationToken,
};

pub struct SystemsMaster<E> {
	results: mpsc::UnboundedReceiver<SystemError<E>>,
	handle: SystemsHandle<E>,
	systems_management_task: JoinHandle<()>,
}

pub struct SystemsMasterBuilder {
	timeout: Option<std::time::Duration>,
	catch_signals: bool,
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
	results_sender: mpsc::UnboundedSender<SystemError<E>>,
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
				results_sender,
			}),
		};

		let systems_handle_clone = systems_handle.clone();

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
			loop {
				tokio::select! {
					biased;
					_ = signal::ctrl_c() => {
						systems_handle.shutdown();
					}
					_ = &mut shutdown_timeout, if self.timeout.is_some() => {
						error!("Graceful stopping timeout reached - aborting tasks");
						aborting = true;
						all_systems.iter_mut().for_each(|f: &mut NamedTask<_>| f.abort());
					}
					new_task_to_listen_for = tasks_receiver.recv() => {
						match new_task_to_listen_for {
							None => {
								if !all_systems.is_empty() {
									trace!("All SystemsHandle have been dropped - cancelling everything and exiting");
									all_systems.iter_mut();
								} else {
									break;
								}
							}
							Some(new_task) => {
								if aborting {
									new_task.abort();
								}
								all_systems.push(new_task);
							}
						}
					}
					task_finished = all_systems.next(), if !all_systems.is_empty() => {
						let res: TaskResult<E> = task_finished.expect("Branch is disabled so we should never get None");
						if let Err(kind) = res.result {
							debug!("System {} errored: {kind}, starting shutdown...", res.name);
							systems_handle.systems.should_stop.cancel();
							// If user doesn't care about results it's fine
							let _: Result<_, mpsc::error::SendError<_>> =
								systems_handle.systems.results_sender.send(SystemError{ system_name: res.name, kind });
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
	pub fn handle(&self) -> &SystemsHandle<E> {
		&self.handle
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

impl<E: Send + fmt::Debug + 'static> SystemsHandle<E> {
	pub fn spawn(&self, system_name: impl Into<String>, f: impl Future<Output = Result<(), E>> + Send + 'static) {
		let name = system_name.into();
		let tasks_sender_guard = self.systems.tasks_sender.load();
		if let Some(tasks_sender) = &*tasks_sender_guard {
			let task = tokio::task::spawn(f);
			tasks_sender
				.send(NamedTask { name, task })
				.expect("Receiving end of the tasks shouldn't have stopped by itself");
		} else {
			// If user doesn't care about results it's fine
			let _: Result<_, mpsc::error::SendError<_>> = self.systems.results_sender.send(SystemError {
				system_name: name,
				kind: SystemErrorKind::NotStarted,
			});
		}
	}
}

impl<E> SystemsHandle<E> {
	pub fn shutdown(&self) {
		self.systems.should_stop.cancel();
		self.systems.tasks_sender.store(None);
	}
}

impl SystemsMasterBuilder {
	pub fn timeout(&mut self, timeout: std::time::Duration) {
		self.timeout = Some(timeout);
	}

	pub fn catch_signals(&mut self) {
		self.catch_signals = true;
	}
}
