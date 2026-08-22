use anyhow::{anyhow, Context, Result};
use openh264::encoder::{Encoder, EncoderConfig};
use openh264::formats::YUVSource;
use windows::{
    core::Interface,
    Win32::{
        Foundation::POINT,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::*,
            Dxgi::{Common::*, *},
        },
        UI::WindowsAndMessaging::{GetCursorInfo, CURSORINFO, CURSOR_SHOWING},
    },
};

pub struct Capturer {
    device:       ID3D11Device,
    context:      ID3D11DeviceContext,
    duplication:  IDXGIOutputDuplication,
    staging:      ID3D11Texture2D,
    pub width:    u32,
    pub height:   u32,
    // Monitor top-left in virtual desktop coords (for cursor offset)
    mon_x: i32,
    mon_y: i32,
    // Cursor shape cache (updated via DXGI GetFramePointerShape)
    cursor_hot:   POINT,
    cursor_shape: Vec<u8>,
    cursor_w:     u32,
    cursor_h:     u32,
    cursor_pitch: u32,
    cursor_type:  i32,
    // H.264 Encoder (optional until first frame)
    encoder: Option<Encoder>,
}

unsafe impl Send for Capturer {}

impl Capturer {
    pub fn new(monitor_index: u32, _quality: u8) -> Result<Self> {
        let (device, context) = create_d3d_device()?;
        let (duplication, width, height) = create_duplication(&device, monitor_index)?;
        let staging = create_staging_texture(&device, width, height)?;
        // Get monitor top-left for cursor coordinate translation
        let (mon_x, mon_y) = crate::input::monitor_offset(monitor_index).unwrap_or((0, 0));
        Ok(Self {
            device, context, duplication, staging, width, height,
            mon_x, mon_y,
            cursor_hot: POINT::default(),
            cursor_shape: Vec::new(),
            cursor_w: 0, cursor_h: 0, cursor_pitch: 0, cursor_type: 0,
            encoder: None,
        })
    }

    pub fn capture_frame(&mut self, scale: f32) -> Result<Option<Vec<u8>>> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        match unsafe { self.duplication.AcquireNextFrame(0, &mut frame_info, &mut resource) } {
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST =>
                anyhow::bail!("DXGI_ERROR_ACCESS_LOST"),
            other => other.context("AcquireNextFrame")?,
        }

        // Keep DXGI shape cache in sync (must call before ReleaseFrame)
        if frame_info.PointerShapeBufferSize > 0 {
            let mut buf = vec![0u8; frame_info.PointerShapeBufferSize as usize];
            let mut info = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
            let mut req = 0u32;
            unsafe {
                let _ = self.duplication.GetFramePointerShape(
                    buf.len() as u32, buf.as_mut_ptr() as _, &mut req, &mut info,
                );
            }
            self.cursor_w     = info.Width;
            self.cursor_h     = info.Height;
            self.cursor_pitch = info.Pitch;
            self.cursor_hot   = POINT { x: info.HotSpot.x as i32, y: info.HotSpot.y as i32 };
            self.cursor_type  = info.Type as i32;
            self.cursor_shape = buf;
        }

        let texture: ID3D11Texture2D = resource.ok_or_else(|| anyhow!("null"))?.cast()?;
        unsafe { self.context.CopyResource(&self.staging, &texture); }
        unsafe { self.duplication.ReleaseFrame()? };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))? };
        let h264 = self.encode_h264(&mapped, scale)?;
        unsafe { self.context.Unmap(&self.staging, 0) };
        Ok(Some(h264))
    }

    fn encode_h264(&mut self, mapped: &D3D11_MAPPED_SUBRESOURCE, scale: f32) -> Result<Vec<u8>> {
        use rayon::prelude::*;

        let pitch = mapped.RowPitch as usize;
        let w = self.width as usize;
        let h = self.height as usize;
        let sw = (w as f32 * scale) as usize;
        let sh = (h as f32 * scale) as usize;
        
        // H.264 requires even dimensions
        let sw = sw & !1;
        let sh = sh & !1;

        let bgra = unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, pitch * h) };
        
        // Allocate Y, U, V planes
        let mut y_plane = vec![0u8; sw * sh];
        let mut u_plane = vec![0u8; (sw / 2) * (sh / 2)];
        let mut v_plane = vec![0u8; (sw / 2) * (sh / 2)];

        // We process 2 rows at a time to easily compute U and V
        y_plane.par_chunks_exact_mut(sw * 2).enumerate().for_each(|(chunk_y, y_chunk)| {
            let sy_top = chunk_y * 2;
            let sy_bot = sy_top + 1;
            let orig_y_top = (sy_top as f32 / scale) as usize;
            let orig_y_bot = (sy_bot as f32 / scale) as usize;

            let u_start = chunk_y * (sw / 2);
            let v_start = u_start;
            // Since we cannot borrow U/V mutably inside this parallel loop safely without split,
            // we will just do a secondary parallel pass for UV, or use raw pointers.
            // Let's just do an unsafe pointer write to U/V planes for max speed.
        });
        
        // Simpler, fully safe approach: process blocks
        let u_ptr = u_plane.as_mut_ptr() as usize;
        let v_ptr = v_plane.as_mut_ptr() as usize;
        
        y_plane.par_chunks_exact_mut(sw * 2).enumerate().for_each(|(chunk_y, y_chunk)| {
            let sy_top = chunk_y * 2;
            let orig_y_top = (sy_top as f32 / scale) as usize;
            let orig_y_bot = ((sy_top + 1) as f32 / scale) as usize;
            
            for sx in (0..sw).step_by(2) {
                let orig_x = (sx as f32 / scale) as usize;
                let orig_x_r = ((sx + 1) as f32 / scale) as usize;
                
                // Top-left pixel
                let mut px = orig_y_top * pitch + orig_x * 4;
                let mut b = bgra[px] as f32;
                let mut g = bgra[px + 1] as f32;
                let mut r = bgra[px + 2] as f32;
                y_chunk[sx] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                let u = (-0.169 * r - 0.331 * g + 0.500 * b + 128.0) as u8;
                let v = (0.500 * r - 0.419 * g - 0.081 * b + 128.0) as u8;
                
                // Top-right
                px = orig_y_top * pitch + orig_x_r * 4;
                b = bgra[px] as f32; g = bgra[px + 1] as f32; r = bgra[px + 2] as f32;
                y_chunk[sx + 1] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                
                // Bottom-left
                px = orig_y_bot * pitch + orig_x * 4;
                b = bgra[px] as f32; g = bgra[px + 1] as f32; r = bgra[px + 2] as f32;
                y_chunk[sw + sx] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                
                // Bottom-right
                px = orig_y_bot * pitch + orig_x_r * 4;
                b = bgra[px] as f32; g = bgra[px + 1] as f32; r = bgra[px + 2] as f32;
                y_chunk[sw + sx + 1] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                
                // Write U/V
                unsafe {
                    let u_dest = (u_ptr as *mut u8).add(chunk_y * (sw / 2) + (sx / 2));
                    let v_dest = (v_ptr as *mut u8).add(chunk_y * (sw / 2) + (sx / 2));
                    *u_dest = u;
                    *v_dest = v;
                }
            }
        });

        // Initialize encoder if needed
        if self.encoder.is_none() {
            let config = EncoderConfig::new().set_bitrate_bps(15_000_000); // 15 Mbps for crisp 1080p60
            self.encoder = Some(Encoder::with_api_config(openh264::OpenH264API::from_source(), config).context("H.264 Encoder init")?);
        }

        let mut yuv = MyYuv {
            y: y_plane, u: u_plane, v: v_plane,
            width: sw, height: sh,
        };

        // Draw cursor directly onto YUV (simplified: just Y channel for now to show cursor)
        let cursor_draw: Option<POINT> = unsafe {
            let mut ci = CURSORINFO {
                cbSize: std::mem::size_of::<CURSORINFO>() as u32,
                ..Default::default()
            };
            if GetCursorInfo(&mut ci).is_ok() && ci.flags == CURSOR_SHOWING {
                let lx = ci.ptScreenPos.x - self.mon_x;
                let ly = ci.ptScreenPos.y - self.mon_y;
                if lx >= 0 && ly >= 0 && lx < w as i32 && ly < h as i32 {
                    Some(POINT { 
                        x: (lx as f32 * scale) as i32, 
                        y: (ly as f32 * scale) as i32 
                    })
                } else { None }
            } else { None }
        };

        if let Some(pos) = cursor_draw {
            if !self.cursor_shape.is_empty() {
                self.draw_cursor_yuv(&mut yuv, pos, scale);
            }
        }

        let bitstream = self.encoder.as_mut().unwrap().encode(&yuv).context("H.264 encode")?;
        Ok(bitstream.to_vec())
    }

    fn draw_cursor_yuv(&self, yuv: &mut MyYuv, pos: POINT, scale: f32) {
        let ox = pos.x - (self.cursor_hot.x as f32 * scale) as i32;
        let oy = pos.y - (self.cursor_hot.y as f32 * scale) as i32;
        let pitch = self.cursor_pitch as usize;
        let fw = yuv.width as usize;
        let fh = yuv.height as usize;

        match self.cursor_type {
            // ── MONOCHROME ─ standard Windows arrow cursor ─────────────────────
            1 => {
                let actual_h = ((self.cursor_h / 2) as f32 * scale) as i32;
                let cw = (self.cursor_w as f32 * scale) as i32;
                for row in 0..actual_h {
                    let orig_row = (row as f32 / scale) as usize;
                    for col in 0..cw {
                        let orig_col = (col as f32 / scale) as usize;
                        let px = ox + col; let py = oy + row;
                        if px < 0 || py < 0 || px >= fw as i32 || py >= fh as i32 { continue; }
                        
                        let bit = 7 - (orig_col % 8) as u32;
                        let and_i = orig_row * pitch + orig_col / 8;
                        let xor_i = (orig_row + (self.cursor_h / 2) as usize) * pitch + orig_col / 8;
                        let and_b = self.cursor_shape.get(and_i).map(|b| (b >> bit) & 1).unwrap_or(1);
                        let xor_b = self.cursor_shape.get(xor_i).map(|b| (b >> bit) & 1).unwrap_or(0);
                        
                        let y_idx = py as usize * fw + px as usize;
                        let uv_idx = (py as usize / 2) * (fw / 2) + (px as usize / 2);
                        
                        match (and_b, xor_b) {
                            (0, 0) => { yuv.y[y_idx] = 16; yuv.u[uv_idx] = 128; yuv.v[uv_idx] = 128; } // Black
                            (0, 1) => { yuv.y[y_idx] = 235; yuv.u[uv_idx] = 128; yuv.v[uv_idx] = 128; } // White
                            (1, 1) => { 
                                yuv.y[y_idx] = 255 - yuv.y[y_idx]; 
                                yuv.u[uv_idx] = 255 - yuv.u[uv_idx]; 
                                yuv.v[uv_idx] = 255 - yuv.v[uv_idx]; 
                            }
                            _ => {} // Transparent
                        }
                    }
                }
            }
            // ── COLOR ─ 32bpp BGRA premultiplied ───────────────────────────────
            2 => {
                let actual_h = (self.cursor_h as f32 * scale) as i32;
                let cw = (self.cursor_w as f32 * scale) as i32;
                for row in 0..actual_h {
                    let orig_row = (row as f32 / scale) as usize;
                    for col in 0..cw {
                        let orig_col = (col as f32 / scale) as usize;
                        let px = ox + col; let py = oy + row;
                        if px < 0 || py < 0 || px >= fw as i32 || py >= fh as i32 { continue; }
                        
                        let ci = orig_row * pitch + orig_col * 4;
                        if ci + 3 >= self.cursor_shape.len() { break; }
                        
                        let a = self.cursor_shape[ci+3] as f32 / 255.0;
                        if a <= 0.01 { continue; }
                        
                        let b = self.cursor_shape[ci] as f32;
                        let g = self.cursor_shape[ci+1] as f32;
                        let r = self.cursor_shape[ci+2] as f32;
                        
                        // cy is pre-multiplied
                        let cy = (0.299 * r + 0.587 * g + 0.114 * b) as f32;
                        
                        let y_idx = py as usize * fw + px as usize;
                        let uv_idx = (py as usize / 2) * (fw / 2) + (px as usize / 2);
                        
                        // Y blend with pre-multiplied alpha
                        yuv.y[y_idx] = (yuv.y[y_idx] as f32 * (1.0 - a) + cy).clamp(0.0, 255.0) as u8;
                        
                        // U/V require un-premultiplied RGB to blend correctly with 128 offset
                        let un_r = r / a;
                        let un_g = g / a;
                        let un_b = b / a;
                        let cu = (-0.169 * un_r - 0.331 * un_g + 0.500 * un_b + 128.0) as f32;
                        let cv = (0.500 * un_r - 0.419 * un_g - 0.081 * un_b + 128.0) as f32;
                        
                        yuv.u[uv_idx] = (yuv.u[uv_idx] as f32 * (1.0 - a) + cu * a).clamp(0.0, 255.0) as u8;
                        yuv.v[uv_idx] = (yuv.v[uv_idx] as f32 * (1.0 - a) + cv * a).clamp(0.0, 255.0) as u8;
                    }
                }
            }
            // ── MASKED COLOR ─ alpha=0 means XOR ──────────────────────────────
            4 => {
                let actual_h = (self.cursor_h as f32 * scale) as i32;
                let cw = (self.cursor_w as f32 * scale) as i32;
                for row in 0..actual_h {
                    let orig_row = (row as f32 / scale) as usize;
                    for col in 0..cw {
                        let orig_col = (col as f32 / scale) as usize;
                        let px = ox + col; let py = oy + row;
                        if px < 0 || py < 0 || px >= fw as i32 || py >= fh as i32 { continue; }
                        
                        let ci = orig_row * pitch + orig_col * 4;
                        if ci + 3 >= self.cursor_shape.len() { break; }
                        
                        let y_idx = py as usize * fw + px as usize;
                        let uv_idx = (py as usize / 2) * (fw / 2) + (px as usize / 2);
                        
                        if self.cursor_shape[ci+3] == 0xFF {
                            let b = self.cursor_shape[ci] as f32;
                            let g = self.cursor_shape[ci+1] as f32;
                            let r = self.cursor_shape[ci+2] as f32;
                            yuv.y[y_idx] = (0.299 * r + 0.587 * g + 0.114 * b) as u8;
                            yuv.u[uv_idx] = (-0.169 * r - 0.331 * g + 0.500 * b + 128.0) as u8;
                            yuv.v[uv_idx] = (0.500 * r - 0.419 * g - 0.081 * b + 128.0) as u8;
                        } else {
                            yuv.y[y_idx] ^= self.cursor_shape[ci+2];
                        }
                    }
                }
            }
            _ => {
                // Fallback: draw a small visible square if type is unknown
                for row in 0..8 {
                    for col in 0..8 {
                        let px = pos.x + col - 4;
                        let py = pos.y + row - 4;
                        if px < 0 || py < 0 || px >= fw as i32 || py >= fh as i32 { continue; }
                        let y_idx = py as usize * fw + px as usize;
                        let uv_idx = (py as usize / 2) * (fw / 2) + (px as usize / 2);
                        yuv.y[y_idx] = 255;
                        yuv.u[uv_idx] = 128;
                        yuv.v[uv_idx] = 128;
                    }
                }
            }
        }
    }
}

struct MyYuv {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    width: usize,
    height: usize,
}

impl YUVSource for MyYuv {
    fn dimensions(&self) -> (usize, usize) { (self.width, self.height) }
    fn strides(&self) -> (usize, usize, usize) { (self.width, self.width / 2, self.width / 2) }
    fn y(&self) -> &[u8] { &self.y }
    fn u(&self) -> &[u8] { &self.u }
    fn v(&self) -> &[u8] { &self.v }
}

fn create_d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let fl = [windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0];
    let mut dev = None; let mut ctx = None; let mut _l = fl[0];
    unsafe { D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, None,
        D3D11_CREATE_DEVICE_FLAG(0), Some(&fl), D3D11_SDK_VERSION,
        Some(&mut dev), Some(&mut _l), Some(&mut ctx))? };
    Ok((dev.ok_or_else(|| anyhow!("no device"))?, ctx.ok_or_else(|| anyhow!("no ctx"))?))
}

fn create_duplication(device: &ID3D11Device, monitor: u32) -> Result<(IDXGIOutputDuplication, u32, u32)> {
    let dxgi: IDXGIDevice   = device.cast()?;
    let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter()? };
    let output: IDXGIOutput   = unsafe { adapter.EnumOutputs(monitor)? };
    let output1: IDXGIOutput1 = output.cast()?;
    let dup = unsafe { output1.DuplicateOutput(device)? };
    let desc = unsafe { dup.GetDesc() };
    let w = desc.ModeDesc.Width;
    let h = desc.ModeDesc.Height;
    Ok((dup, w, h))
}

fn create_staging_texture(device: &ID3D11Device, w: u32, h: u32) -> Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w, Height: h, MipLevels: 1, ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..Default::default()
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex))? };
    tex.ok_or_else(|| anyhow!("null texture"))
}
