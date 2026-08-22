/// Controls the virtual display driver by itsmikethetech/Virtual-Display-Driver.
/// The driver exposes a named pipe: `\\.\pipe\VirtualDisplayDriver`
/// with a simple JSON protocol.
///
/// If the driver is not installed, this module falls back gracefully and the
/// user picks an existing monitor index via CLI.
use anyhow::{Context, Result};
use std::io::{Read, Write};
use std::time::Duration;

pub struct VirtualDisplay {
    monitor_id: Option<u32>,
}

impl VirtualDisplay {
    /// Try to create a virtual display at the given resolution / refresh rate.
    /// Returns Ok(Self) even if the driver is unavailable (monitor_id = None).
    pub fn create(width: u32, height: u32, refresh: u32) -> Result<Self> {
        match Self::try_create(width, height, refresh) {
            Ok(id) => {
                log::info!("Virtual display created (id={id}) at {width}x{height}@{refresh}Hz");
                Ok(Self { monitor_id: Some(id) })
            }
            Err(e) => {
                log::warn!("VDD driver not available ({e}). Using existing monitor — pass --monitor <index>.");
                Ok(Self { monitor_id: None })
            }
        }
    }

    fn try_create(width: u32, height: u32, refresh: u32) -> Result<u32> {
        let mut pipe = Self::open_pipe()?;

        // Send Add command
        let cmd = serde_json::json!({
            "Add": [{ "width": width, "height": height, "refresh_rates": [refresh] }]
        });
        let cmd_str = serde_json::to_string(&cmd)?;
        pipe.write_all(cmd_str.as_bytes()).context("pipe write")?;
        pipe.flush()?;

        // Read response — driver echoes back with assigned IDs
        let mut buf = [0u8; 512];
        let n = pipe.read(&mut buf).context("pipe read")?;
        let response = std::str::from_utf8(&buf[..n])?;
        log::debug!("VDD response: {response}");

        // Parse monitor id from response JSON array (driver returns list of active monitors)
        let ids: serde_json::Value = serde_json::from_str(response)
            .context("invalid VDD response")?;
        let id = ids
            .as_array()
            .and_then(|a| a.last())
            .and_then(|v| v.as_u64())
            .context("no monitor id in response")? as u32;

        Ok(id)
    }

    fn open_pipe() -> Result<std::fs::File> {
        // Named pipes on Windows are accessible via std::fs on the \\.\pipe\ path.
        for _ in 0..10 {
            match std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(r"\\.\pipe\VirtualDisplayDriver")
            {
                Ok(f) => return Ok(f),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
        anyhow::bail!("Cannot open VDD named pipe")
    }

    pub fn is_active(&self) -> bool {
        self.monitor_id.is_some()
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        if let Some(id) = self.monitor_id {
            if let Ok(mut pipe) = Self::open_pipe() {
                let cmd = serde_json::json!({ "Remove": [id] });
                if let Ok(s) = serde_json::to_string(&cmd) {
                    let _ = pipe.write_all(s.as_bytes());
                }
                log::info!("Virtual display {id} removed.");
            }
        }
    }
}
