//! Total CPU and RAM of the machine, pushed to the widget every 2 seconds.

use std::thread;
use std::time::Duration;

use serde_json::json;
use sysinfo::System;
use tauri::{AppHandle, Emitter};

const INTERVAL: Duration = Duration::from_secs(2);

pub fn start(app: AppHandle) {
    thread::spawn(move || {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        loop {
            thread::sleep(INTERVAL);
            sys.refresh_cpu_usage();
            sys.refresh_memory();
            let _ = app.emit_to(
                "main",
                "sys",
                json!({
                    "cpu": sys.global_cpu_usage(),
                    "memUsed": sys.used_memory(),
                    "memTotal": sys.total_memory(),
                }),
            );
        }
    });
}
