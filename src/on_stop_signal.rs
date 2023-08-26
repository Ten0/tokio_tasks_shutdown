/// Waits for a signal that requests a graceful shutdown, like SIGTERM or SIGINT.
#[cfg(unix)]
pub(crate) async fn on_stop_signal() {
	use tokio::signal::unix::{signal, SignalKind};

	let mut signal_terminate = signal(SignalKind::terminate()).unwrap();
	let mut signal_interrupt = signal(SignalKind::interrupt()).unwrap();

	tokio::select! {
		_ = signal_terminate.recv() => log::trace!("Received SIGTERM"),
		_ = signal_interrupt.recv() => log::trace!("Received SIGINT"),
	};
}

/// Waits for a signal that requests a graceful shutdown, Ctrl-C
#[cfg(windows)]
pub(crate) async fn on_stop_signal() {
	use tokio::signal::ctrl_c;

	ctrl_c().await.unwrap();
	log::trace!("Received Ctrl+C");
}
