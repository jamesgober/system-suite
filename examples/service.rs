//! A small service: boots, runs one supervised worker that watches the
//! urgent line, and stops in order on a signal.
//!
//! ```text
//! cargo run --example service
//! ```
//!
//! Set `SERVICE_EXAMPLE_CONFIG` to name a configuration file, for example
//! one with `[kernel] memory_watch = "1s"` to run the memory watermark.
//! `SERVICE_FILL_MB` holds that much memory for the life of the process, so
//! the watermark has something to see.

use std::{env, process::ExitCode, time::Duration};

use system_suite::{System, runtime::Worker};

fn main() -> ExitCode {
    let system = match System::builder("service-example").start() {
        Ok(system) => system,
        Err(error) => return error.report(),
    };
    let ballast = vec![1_u8; fill_bytes()];

    let worker = Worker::new("load").spawn(system.runtime(), |cx| {
        let mut watch = cx.urgent().watch();
        while !cx.should_stop() {
            if let Some(level) = watch.changed() {
                println!("urgent level: {level}");
            }
            cx.park(Some(Duration::from_millis(50)));
        }
        Ok(())
    });
    if let Err(error) = worker {
        return error.report();
    }

    println!("ready");
    let code = system.run();
    drop(ballast);
    code
}

/// `SERVICE_FILL_MB` in bytes; none when it is unset or not a number.
fn fill_bytes() -> usize {
    env::var("SERVICE_FILL_MB")
        .ok()
        .and_then(|megabytes| megabytes.parse::<usize>().ok())
        .map_or(0, |megabytes| megabytes.saturating_mul(1 << 20))
}
