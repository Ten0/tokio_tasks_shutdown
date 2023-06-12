# tokio_tasks_shutdown

**An idiomatic implementation of serde/avro (de)serialization**

[![Crates.io](https://img.shields.io/crates/v/tokio_tasks_shutdown.svg)](https://crates.io/crates/tokio_tasks_shutdown)
[![License](https://img.shields.io/github/license/Ten0/tokio_tasks_shutdown)](LICENSE)

Easily manage and gracefully shutdown tokio tasks while monitoring their return results

# Example

```rust
use {std::time::Duration, tokio::time::sleep, tokio_tasks_shutdown::*};

// By default this will catch signals.
// You may have your tasks return your own error type.
let tasks = TasksBuilder::default()
	.timeout(Duration::from_secs(2))
	.build::<anyhow::Error>();

// Let's simulate a Ctrl+C after some time
let tasks_handle = tasks.handle();
tokio::task::spawn(async move {
	sleep(Duration::from_millis(150)).await;
	tasks_handle.start_shutdown();
});

// Spawn tasks
tasks
	.spawn("gracefully_shutting_down_task", |tasks_handle| async move {
		loop {
			tokio::select! {
				biased;
				_ = tasks_handle.on_shutdown() => {
					// We have been kindly asked to shutdown, let's exit
					break;
				}
				_ = sleep(Duration::from_millis(100)) => {
					// Simulating another task running concurrently, e.g. listening on a channel...
				}
			}
		}
		Ok(())
	})
	.unwrap();
// Note that calls can be chained since `spawn` returns `&TasksHandle`

// Let's make sure there were no errors
tasks.join_all().await.unwrap();
```

In this example, the task will have run one loop already (sleep has hit at t=100ms) when asked for graceful
shutdown at t=150ms, which will immediately make it gracefully shut down.
