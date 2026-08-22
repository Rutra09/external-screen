use serde::{Deserialize, Serialize};

// ─── Framing protocol (TCP, both directions) ─────────────────────────────────
//
//  ┌──────────┬───────────────┬─────────────────┐
//  │  type u8 │  length u32BE │  payload [u8]   │
//  └──────────┴───────────────┴─────────────────┘
//
//  Type codes:
//    0x01  HANDSHAKE  server→client  JSON HandshakeInfo
//    0x02  FRAME      server→client  raw JPEG bytes
//    0x03  TOUCH      client→server  JSON TouchEvent

pub const MSG_HANDSHAKE: u8 = 0x01;
pub const MSG_FRAME: u8     = 0x02;
pub const MSG_TOUCH: u8     = 0x03;

/// Sent once on connect so Android knows display dimensions.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HandshakeInfo {
    pub width:  u32,
    pub height: u32,
    pub fps:    u32,
}

/// Touch/pointer event from Android.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TouchEvent {
    /// "down" | "move" | "up"
    pub action: String,
    /// Normalized 0.0–1.0 relative to display size
    pub x: f32,
    pub y: f32,
    /// Pointer id (multi-touch future-proofing, use 0 for now)
    pub id: u32,
}
