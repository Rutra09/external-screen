// Narzędzie diagnostyczne — wypisuje wszystkie dostępne monitory DXGI
// Uruchom: cargo run --bin list-monitors
//
// Użycie: sprawdź który indeks to Twój wirtualny display po instalacji VDD
use windows::{
    core::Interface,
    Win32::Graphics::{
        Direct3D::D3D_DRIVER_TYPE_HARDWARE,
        Direct3D11::*,
        Dxgi::*,
    },
};

fn main() -> anyhow::Result<()> {
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
    let dxgi: IDXGIDevice   = device.cast()?;
    let adapter: IDXGIAdapter = unsafe { dxgi.GetAdapter()? };

    println!("Dostępne monitory:");
    println!("{:<8} {:<20} {}", "Indeks", "Rozdzielczość", "Nazwa");
    println!("{}", "-".repeat(55));

    for i in 0u32.. {
        let output = match unsafe { adapter.EnumOutputs(i) } {
            Ok(o) => o,
            Err(_) => break,
        };
        let desc = unsafe { output.GetDesc() };
        match desc {
            Ok(d) => {
                let w = d.DesktopCoordinates.right  - d.DesktopCoordinates.left;
                let h = d.DesktopCoordinates.bottom - d.DesktopCoordinates.top;
                // DeviceName is [u16; 32]
                let name: String = d.DeviceName.iter()
                    .take_while(|&&c| c != 0)
                    .map(|&c| char::from_u32(c as u32).unwrap_or('?'))
                    .collect();
                println!("{:<8} {:<20} {}", i, format!("{}x{}", w, h), name);
            }
            Err(_) => {
                println!("{:<8} (błąd GetDesc)", i);
            }
        }
    }
    Ok(())
}
