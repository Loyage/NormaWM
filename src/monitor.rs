//! Background monitoring for the compositor control plane.
//!
//! This module intentionally has no UI. The monitor starts with `normawm`,
//! keeps lightweight runtime counters, and enriches `norma msg status`
//! responses through the existing local control socket.

use std::time::{Duration, Instant};

use crate::control::{ControlMonitorInfo, ControlStatus};

const STATUS_BROADCAST_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct BackgroundMonitor {
    started_at: Instant,
    last_status_broadcast: Instant,
    commands_seen: u64,
    status_broadcasts: u64,
}

impl BackgroundMonitor {
    pub fn start() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_status_broadcast: now,
            commands_seen: 0,
            status_broadcasts: 0,
        }
    }

    pub fn record_command(&mut self) {
        self.commands_seen += 1;
    }

    pub fn should_broadcast_status(&self, forced: bool) -> bool {
        forced || self.last_status_broadcast.elapsed() >= STATUS_BROADCAST_INTERVAL
    }

    pub fn record_status_broadcast(&mut self) {
        self.status_broadcasts += 1;
        self.last_status_broadcast = Instant::now();
    }

    pub fn enrich_status(&self, status: &mut ControlStatus) {
        status.monitor = Some(ControlMonitorInfo {
            uptime_ms: self.started_at.elapsed().as_millis(),
            commands_seen: self.commands_seen,
            status_broadcasts: self.status_broadcasts,
        });
    }
}
