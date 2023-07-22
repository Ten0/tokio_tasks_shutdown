# tokio_tasks_shutdown

**Easily manage and gracefully shutdown tokio tasks while monitoring their return results**

[![Crates.io](https://img.shields.io/crates/v/tokio_tasks_shutdown.svg)](https://crates.io/crates/tokio_tasks_shutdown)
[![License](https://img.shields.io/github/license/Ten0/tokio_tasks_shutdown)](LICENSE)

## Example

```rust
use {std::time::Duration, tokio::time::sleep, tokio_tasks_shutdown::*};

// By default this will catch Ctrl+C.
// You may have your tasks return your own error type.
let tasks: TasksMainHandle<anyhow::Error> = TasksBuilder::default()
	.timeouts(Some(Duration::from_secs(2)), Some(Duration::from_millis(500)))
	.build();

// Spawn tasks
tasks
	.spawn("gracefully_shutting_down_task", |tasks_handle| async move {
		loop {
			match tasks_handle
				.on_shutdown_or({
					// Simulating another future running concurrently,
					// e.g. listening on a channel...
					sleep(Duration::from_millis(100))
				})
				.await
			{
				ShouldShutdownOr::ShouldShutdown => {
					// We have been kindly asked to shutdown, let's exit
					break;
				}
				ShouldShutdownOr::ShouldNotShutdown(res) => {
					// Got result of channel listening
				}
			}
		}
		Ok(())
		// Note that if a task were to error, graceful shutdown would be initiated.
		// This behavior can be disabled.
	})
	.unwrap();
// Note that calls can be chained since `spawn` returns `&TasksHandle`

// Let's simulate a Ctrl+C after some time
let tasks_handle: TasksHandle<_> = tasks.handle();
tokio::task::spawn(async move {
	sleep(Duration::from_millis(150)).await;
	tasks_handle.start_shutdown();
});

// Let's make sure there were no errors
tasks.join_all().await.unwrap();

// Make sure we have shut down when expected
assert!(
	test_duration > Duration::from_millis(145) && test_duration < Duration::from_millis(155)
);
```

In this example, the task will have run one loop already (sleep has hit at t=100ms) when asked for graceful
shutdown at t=150ms, which will immediately make it gracefully shut down.

## Similar crates

This crate is inspired from [`tokio-graceful-shutdown`](https://github.com/Finomnis/tokio-graceful-shutdown/).
I ran into issues using it where it would sometimes freeze when tasks
were supposed to timeout and that was very hard to investigate because there is a lot of code, so I figured the easiest
way to fix it would be to just rewrite a simpler version of it from scratch.

Compared to `tokio-graceful-shutdown`, it has 3x less code, no subsystem concept
(any shutdown is global on a `TasksMainHandle`),
[more flexible control](https://github.com/Finomnis/tokio-graceful-shutdown/issues/54),
and a (hopefully) more reliable shutdown mechanism
(notably it can survive tasks accidentally blocking - although I don't think that was my original issue).
