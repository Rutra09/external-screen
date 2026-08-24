/// Translates normalized (0-1) touch coordinates from Android
/// into absolute mouse events on Windows via SendInput.
use anyhow::Result;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

use crate::protocol::TouchEvent;

pub fn handle_touch(event: &TouchEvent, display_x: i32, display_y: i32, w: u32, h: u32) {
    // Scale normalized coords to absolute screen coords of the virtual display
    let abs_x = display_x + (event.x * w as f32) as i32;
    let abs_y = display_y + (event.y * h as f32) as i32;

    // Windows absolute coords are 0-65535 across the full virtual desktop
    // (origin can be negative when monitors sit left/above the primary one)
    let (vs_x, vs_y, vs_w, vs_h) = virtual_desktop_rect();
    let norm_x = (((abs_x - vs_x) as f64 / vs_w as f64) * 65535.0) as i32;
    let norm_y = (((abs_y - vs_y) as f64 / vs_h as f64) * 65535.0) as i32;

    let flags_base = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE | MOUSEEVENTF_VIRTUALDESK;

    let button_flags = match event.action.as_str() {
        "down" => MOUSEEVENTF_LEFTDOWN,
        "up"   => MOUSEEVENTF_LEFTUP,
        _      => MOUSEEVENTF_MOVE,
    };

    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx:          norm_x,
                dy:          norm_y,
                dwFlags:     flags_base | button_flags,
                dwExtraInfo: 0,
                mouseData:   0,
                time:        0,
            },
        },
    };

    unsafe {
        SendInput(&[input], std::mem::size_of::<INPUT>() as i32);
    }
}

fn virtual_desktop_rect() -> (i32, i32, i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Returns the top-left corner of a monitor by DXGI index (for coordinate offset).
pub fn monitor_offset(monitor_index: u32) -> Result<(i32, i32)> {
    use windows::{
        core::Interface,
        Win32::Graphics::{
            Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            Direct3D11::*,
            Dxgi::*,
        },
    };

    let feature_levels = [windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0];
    let mut device = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    let device = device.unwrap();
    let dxgi: IDXGIDevice     = device.cast()?;
    let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter()? };
    let output: IDXGIOutput   = unsafe { adapter.EnumOutputs(monitor_index)? };
    let desc = unsafe { output.GetDesc()? };
    Ok((desc.DesktopCoordinates.left, desc.DesktopCoordinates.top))
}
