use {
	anyhow::{bail, Error as InternalError},
	log::info,
	std::time::Duration,
	tokio::time::sleep,
};

use tokio_tasks_shutdown::*;

fn init_log() {
	match simple_logger::SimpleLogger::new().init() {
		Ok(()) => {}
		Err(e) => {
			let _: log::SetLoggerError = e;
		}
	}
}

#[tokio::test]
async fn simple() {
	init_log();

	let start = std::time::Instant::now();
	let master = TasksBuilder::default()
		.dont_catch_signals()
		.timeouts(Some(Duration::from_secs(2)), Some(Duration::from_millis(500)))
		.build::<InternalError>();
	let handle = master.handle();
	tokio::task::spawn(async move {
		sleep(Duration::from_millis(500)).await;
		handle.start_shutdown();
	});
	master
		.spawn("S1", |handle| async move {
			info!("Started task");
			sleep(Duration::from_secs(1)).await;
			info!("Slept");
			handle.on_shutdown().await;
			info!("Got shutdown msg");
			Ok(())
		})
		.unwrap();
	master.join_all_with(|e| panic!("{e}")).await.unwrap();
	let elapsed = start.elapsed();
	assert!(elapsed > Duration::from_millis(900) && elapsed < Duration::from_millis(1100));
}

#[tokio::test(flavor = "multi_thread")]
async fn more_complex() {
	init_log();

	// Apply the workaround described at https://github.com/tokio-rs/tokio/issues/4730#issuecomment-1147165954
	// to make `task_abort_timeout` 100% reliable
	let rt_handle = tokio::runtime::Handle::current();
	std::thread::spawn(move || loop {
		std::thread::sleep(Duration::from_millis(10));
		rt_handle.spawn(std::future::ready(()));
	});

	let start = std::time::Instant::now();
	let master = TasksBuilder::default()
		.dont_catch_signals()
		.timeouts(Some(Duration::from_secs(2)), Some(Duration::from_millis(500)))
		.build::<InternalError>();
	let handle = master.handle();
	tokio::task::spawn(async move {
		sleep(Duration::from_millis(500)).await;
		handle.start_shutdown();
	});
	master
		.spawn("1_regular_task", |handle| async move {
			info!("1_regular_task");
			sleep(Duration::from_secs(1)).await;
			info!("1_regular_task Slept");
			handle.on_shutdown().await;
			info!("1_regular_task Got shutdown msg");
			Ok(())
		})
		.unwrap()
		.spawn("2_user_error", |handle| async move {
			info!("2_user_error Started task");
			handle.on_shutdown().await;
			info!("2_user_error Got shutdown msg");
			sleep(Duration::from_millis(100)).await;
			bail!("O no")
		})
		.unwrap()
		.spawn("3_task_cancelled", |handle| async move {
			info!("3_task_cancelled Started task");
			sleep(Duration::from_secs(1)).await;
			info!("3_task_cancelled Slept");
			handle.on_shutdown().await;
			info!("3_task_cancelled Got shutdown msg");
			sleep(Duration::from_secs(3)).await;
			Ok(())
		})
		.unwrap()
		.spawn("4_evil_blocking_task_left_dangling", |handle| async move {
			info!("4_evil_blocking_task_left_dangling Started task");
			sleep(Duration::from_secs(1)).await;
			info!("4_evil_blocking_task_left_dangling Slept");
			handle.on_shutdown().await;
			info!("4_evil_blocking_task_left_dangling Got shutdown msg");
			// This has the desired effect of letting the task dangle only with
			// #[tokio::test(flavor = "multi_thread")]
			std::thread::sleep(Duration::from_secs(4));
			Ok(())
		})
		.unwrap();
	let mut errors = Vec::new();
	let _: Result<(), results::AtLeastOneTaskErrored> = master.join_all_with(|e| errors.push(e)).await;
	let elapsed = start.elapsed();
	dbg!(elapsed);
	errors.sort_by(|a, b| a.task_name().cmp(b.task_name()));
	let patterns = &[
		r#"Task 2_user_error errored: User error: O no"#,
		r#"Task 3_task_cancelled errored: Tokio join error: task \d+ was cancelled"#,
		r#"Task 4_evil_blocking_task_left_dangling errored: Cancel timeout exceeded - left task dangle"#,
	];
	if patterns.len() != errors.len()
		|| !errors
			.iter()
			.zip(patterns)
			.all(|(e, pat)| regex::Regex::new(pat).unwrap().is_match(&e.to_string()))
	{
		dbg!(errors.iter().map(|e| e.to_string()).collect::<Vec<_>>());
		panic!("Errors mismatch")
	}
	assert!(elapsed > Duration::from_millis(2900) && elapsed < Duration::from_millis(3100));
}
