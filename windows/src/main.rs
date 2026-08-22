mod capture;
mod input;
mod protocol;
mod server;
mod virtual_display;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Virtual Screen Server — streams a Windows display to an Android device.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// TCP port to listen on
    #[arg(short, long, default_value_t = 9999)]
    port: u16,

    /// Monitor index to capture (0 = primary, 1 = secondary…)
    #[arg(short, long, default_value_t = 2)]
    monitor: u32,

    /// Create a virtual display via Virtual Display Driver (IDD) before capturing.
    /// Requires itsmikethetech/Virtual-Display-Driver to be installed.
    #[arg(long)]
    virtual_display: bool,

    /// Virtual display width (only used with --virtual-display)
    #[arg(long, default_value_t = 1920)]
    width: u32,

    /// Virtual display height (only used with --virtual-display)
    #[arg(long, default_value_t = 1080)]
    height: u32,

    /// Virtual display refresh rate (only used with --virtual-display)
    #[arg(long, default_value_t = 60)]
    refresh: u32,

    /// JPEG quality 1-100 (lower = faster / smaller)
    #[arg(short, long, default_value_t = 65)]
    quality: u8,

    /// Downscale factor 0.1–1.0 (0.5 = half resolution, 4x less data — recommended for Wi-Fi)
    #[arg(long, default_value_t = 0.5)]
    scale: f32,

    /// Target frames per second
    #[arg(long, default_value_t = 30)]
    fps: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let args = Args::parse();

    // Optionally create a virtual display
    let _vd = if args.virtual_display {
        log::info!("Creating virtual display {}x{}@{}Hz…", args.width, args.height, args.refresh);
        Some(virtual_display::VirtualDisplay::create(args.width, args.height, args.refresh)?)
    } else {
        None
    };

    // Short sleep so the OS registers the new display before we try to capture it
    if args.virtual_display {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    log::info!("Initializing DXGI capture on monitor {}…", args.monitor);
    let capturer = capture::Capturer::new(args.monitor, args.quality)
        .map_err(|e| anyhow::anyhow!("Capture init failed: {e}\nHint: check --monitor index or install VDD driver"))?;

    let scale = args.scale.clamp(0.1, 1.0);
    log::info!(
        "Capturing {}x{} → scale {:.1} → {}x{} at H.264 15Mbps @ {}fps",
        capturer.width, capturer.height, scale,
        (capturer.width as f32 * scale) as u32,
        (capturer.height as f32 * scale) as u32,
        args.fps
    );

    let capturer = Arc::new(Mutex::new(capturer));

    // Block on the server (Ctrl+C drops _vd, removing virtual display)
    server::run(args.port, capturer, args.monitor, args.quality, scale, args.fps).await
}
