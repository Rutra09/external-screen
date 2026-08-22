use anyhow::Result;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
    time::{interval, Duration},
};
use crate::{capture::Capturer, input, protocol::*};

pub async fn run(
    port: u16,
    capturer: Arc<Mutex<Capturer>>,
    monitor_index: u32,
    quality: u8,
    scale: f32,
    fps: u32,
) -> Result<()> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    log::info!("Listening on 0.0.0.0:{port}  (LAN or ADB reverse tcp:{port} tcp:{port})");

    loop {
        let (stream, addr) = listener.accept().await?;
        log::info!("Client connected: {addr}");
        stream.set_nodelay(true)?;
        let capturer = capturer.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(stream, capturer, monitor_index, quality, scale, fps).await {
                log::warn!("Client {addr} disconnected: {e}");
            }
        });
    }
}

async fn handle_client(
    stream: tokio::net::TcpStream,
    capturer: Arc<Mutex<Capturer>>,
    monitor_index: u32,
    quality: u8,
    scale: f32,
    fps: u32,
) -> Result<()> {
    let (mut reader, mut writer) = stream.into_split();

    let (w, h) = {
        let c = capturer.lock().await;
        let sw = (c.width  as f32 * scale) as u32;
        let sh = (c.height as f32 * scale) as u32;
        (sw, sh)
    };
    let hs = serde_json::to_vec(&HandshakeInfo { width: w, height: h, fps })?;
    send_msg(&mut writer, MSG_HANDSHAKE, &hs).await?;

    let (mon_x, mon_y) = input::monitor_offset(monitor_index).unwrap_or((0, 0));
    let (w2, h2) = (w, h);

    tokio::spawn(async move {
        loop {
            match recv_msg(&mut reader).await {
                Ok((MSG_TOUCH, payload)) => {
                    if let Ok(ev) = serde_json::from_slice::<TouchEvent>(&payload) {
                        input::handle_touch(&ev, mon_x, mon_y, w2, h2);
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let frame_ms = 1000 / fps.max(1);
    let mut tick = interval(Duration::from_millis(frame_ms as u64));
    loop {
        tick.tick().await;

        let frame_opt = {
            let mut c = capturer.lock().await;
            match c.capture_frame(scale) {
                Ok(f) => f,
                Err(e) if e.to_string().contains("ACCESS_LOST") => {
                    log::warn!("ACCESS_LOST — recreating capturer…");
                    drop(c);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    match Capturer::new(monitor_index, quality) {
                        Ok(nc) => { *capturer.lock().await = nc; }
                        Err(e2) => log::warn!("Capturer recreate failed: {e2}"),
                    }
                    continue;
                }
                Err(e) => return Err(e),
            }
        };

        if let Some(jpeg) = frame_opt {
            send_msg(&mut writer, MSG_FRAME, &jpeg).await?;
        }
    }
}

async fn send_msg(w: &mut (impl AsyncWriteExt + Unpin), t: u8, p: &[u8]) -> Result<()> {
    w.write_u8(t).await?;
    w.write_u32(p.len() as u32).await?;
    w.write_all(p).await?;
    Ok(())
}

async fn recv_msg(r: &mut (impl AsyncReadExt + Unpin)) -> Result<(u8, Vec<u8>)> {
    let t = r.read_u8().await?;
    let n = r.read_u32().await? as usize;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok((t, buf))
}
