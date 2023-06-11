use {
	anyhow::{bail, Error as InternalError},
	log::info,
	std::time::Duration,
	tokio::time::sleep,
};

use async_systems_shutdown::*;

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
	let master = SystemsMasterBuilder::default()
		.dont_catch_signals()
		.timeout(Duration::from_secs(2))
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
	master.with_errors(|e| panic!("{e}")).await.unwrap();
	let elapsed = start.elapsed();
	assert!(elapsed > Duration::from_millis(900) && elapsed < Duration::from_millis(1100));
}

#[tokio::test]
async fn more_complex() {
	init_log();

	let start = std::time::Instant::now();
	let master = SystemsMasterBuilder::default()
		.dont_catch_signals()
		.timeout(Duration::from_secs(2))
		.build::<InternalError>();
	let handle = master.handle();
	tokio::task::spawn(async move {
		sleep(Duration::from_millis(500)).await;
		handle.start_shutdown();
	});
	master
		.spawn("S1", |handle| async move {
			info!("S1 Started task");
			sleep(Duration::from_secs(1)).await;
			info!("S1 Slept");
			handle.on_shutdown().await;
			info!("S1 Got shutdown msg");
			Ok(())
		})
		.unwrap()
		.spawn("S2", |handle| async move {
			info!("S2 Started task");
			handle.on_shutdown().await;
			info!("S2 Got shutdown msg");
			sleep(Duration::from_millis(100)).await;
			bail!("O no")
		})
		.unwrap()
		.spawn("S3", |handle| async move {
			info!("S3 Started task");
			sleep(Duration::from_secs(1)).await;
			info!("S3 Slept");
			handle.on_shutdown().await;
			info!("S3 Got shutdown msg");
			sleep(Duration::from_secs(3)).await;
			Ok(())
		})
		.unwrap();
	let mut errors = Vec::new();
	let _: Result<(), AtLeastOneSystemErrored> = master.with_errors(|e| errors.push(e)).await;
	let elapsed = start.elapsed();
	errors.sort_by(|a, b| a.system_name().cmp(b.system_name()));
	let patterns = &[
		r#"System S2 errored: User error: O no"#,
		r#"System S3 errored: Tokio join error: task \d+ was cancelled"#,
	];
	assert_eq!(patterns.len(), errors.len());
	assert!(errors
		.iter()
		.zip(patterns)
		.all(|(e, pat)| regex::Regex::new(pat).unwrap().is_match(&e.to_string())));
	assert!(elapsed > Duration::from_millis(2400) && elapsed < Duration::from_millis(2600));
}
