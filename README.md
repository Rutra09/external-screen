# Virtual Screen

Android jako wirtualny monitor dla Windows.  
Połączenie przez **LAN** lub **USB (ADB)**.

---

## Jak to działa

```
Windows (Rust server)
  └─ tworzy virtual display (IDD driver)
  └─ DXGI Desktop Duplication → JPEG frames
  └─ raw TCP  [u8 type][u32 len][payload]
       │
       ├── LAN (Wi-Fi / Ethernet)
       └── USB via adb reverse

Android (Kotlin app)
  └─ java.net.Socket → odbiera JPEG
  └─ SurfaceView → renderuje ramki
  └─ touch events → wysyła z powrotem do Windows
```

---

## Wymagania

### Windows
- **Rust** (https://rustup.rs)
- **Virtual Display Driver** (IDD) — itsmikethetech  
  → https://github.com/itsmikethetech/Virtual-Display-Driver/releases  
  Pobierz i zainstaluj `.inf` (prawy klik → Zainstaluj)

### Android
- **Android Studio** z Android SDK
- Telefon z Androidem 8.0+ (API 26)

---

## Build & uruchomienie

### Windows server

```powershell
cd windows
cargo build --release

# Z wirtualnym displayem (wymaga VDD):
.\target\release\virtual-screen-server.exe --virtual-display --width 1920 --height 1080

# Albo bez VDD — przechwytuje istniejący monitor (np. monitor #1):
.\target\release\virtual-screen-server.exe --monitor 1
```

Flagi:
| Flag | Domyślnie | Opis |
|------|-----------|------|
| `--port` | 9999 | Port TCP |
| `--monitor` | 1 | Indeks monitora (0=primary) |
| `--virtual-display` | off | Utwórz wirtualny display przez VDD |
| `--width/--height/--refresh` | 1920x1080@60 | Rozdzielczość virtual display |
| `--quality` | 75 | JPEG quality (1-100) |

### Android app

```bash
cd android
# Otwórz w Android Studio i kliknij Run
# Albo:
./gradlew installDebug
```

---

## Połączenie

### LAN (Wi-Fi / Ethernet)
1. Uruchom server na Windows
2. W aplikacji Android: ⚙ → wpisz IP komputera (np. `192.168.1.5`) i port `9999`
3. Kliknij **Connect**

### USB
1. Podłącz kabel USB, włącz debugowanie USB
2. W PowerShell:
   ```powershell
   adb reverse tcp:9999 tcp:9999
   ```
3. W aplikacji Android: wpisz `127.0.0.1` jako host
4. Kliknij **Connect**

---

## Troubleshooting

| Problem | Rozwiązanie |
|---------|-------------|
| `Capture init failed` | Sprawdź `--monitor` index — uruchom bez `--virtual-display` i spróbuj 0, 1, 2 |
| VDD pipe error | Zainstaluj Virtual Display Driver i poczekaj chwilę po instalacji |
| Czarny ekran na Android | Windows Defender / firewall — odblokuj port 9999 TCP |
| Wysokie opóźnienie | Użyj USB zamiast Wi-Fi, zmniejsz `--quality` (np. 50) |
