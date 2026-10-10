//! Ambilight: screen-edge colours for the strip. Everything here is plain
//! Rust without Win32; screen capture lives in `crate::capture`.

pub mod color;
pub mod image;
pub mod layout;
pub mod letterbox;
pub mod pipeline;
pub mod rate;
pub mod smooth;
