use {
	futures::prelude::*,
	std::{
		pin::Pin,
		sync::atomic::{AtomicBool, Ordering},
		task::{Context, Poll},
	},
	tokio::sync::Notify,
};

#[derive(Default)]
pub(crate) struct ShouldShutdown {
	atomic: AtomicBool,
	notify: Notify,
}

impl ShouldShutdown {
	pub(crate) fn start_shutdown(&self) {
		self.atomic.store(true, std::sync::atomic::Ordering::SeqCst);
		self.notify.notify_waiters();
	}

	pub(crate) fn start_shutdown_check_was(&self) -> bool {
		let was_shutting_down = self.atomic.swap(true, std::sync::atomic::Ordering::SeqCst);
		self.notify.notify_waiters();
		was_shutting_down
	}

	pub(crate) fn on_shutdown<'a>(&'a self) -> OnShutdownFuture<'a> {
		OnShutdownFuture {
			should_shutdown: self,
			future: self.notify.notified(),
		}
	}

	pub(crate) fn should_shutdown(&self) -> bool {
		self.atomic.load(Ordering::Relaxed)
	}
}

// This is a copy of tokio_util's cancellation_token, except that it doesn't have the tree abstraction,
// so no mutex to check for cancellation -> faster

pin_project_lite::pin_project! {
	/// A Future that is resolved once the corresponding [`CancellationToken`]
	/// is cancelled.
	#[must_use = "futures do nothing unless polled"]
	pub(crate) struct OnShutdownFuture<'a> {
		should_shutdown: &'a ShouldShutdown,
		#[pin]
		future: tokio::sync::futures::Notified<'a>,
	}
}

impl<'a> Future for OnShutdownFuture<'a> {
	type Output = ();

	fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
		let mut this = self.project();

		loop {
			if this.should_shutdown.atomic.load(Ordering::SeqCst) {
				return Poll::Ready(());
			}

			// No wakeups can be lost here because there is always a load on
			// `atomic` between the creation of the future and the call to
			// `poll`, and the code that sets the cancelled flag does so before
			// waking the `Notified`.
			if this.future.as_mut().poll(cx).is_pending() {
				return Poll::Pending;
			}

			// https://github.com/tokio-rs/tokio/pull/4652#issuecomment-1646639198
			this.future.set(this.should_shutdown.notify.notified());
		}
	}
}
