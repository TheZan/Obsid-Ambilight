//! Obsid Ambilight: local control of the Yeelight Obsid (Chameleon2) RGBIC strip.
//!
//! Stage 0 (protocol proof of concept) contains only the device-facing layers:
//! [`protocol`] talks to the strip over the Yeelight UDP control protocol,
//! [`discovery`] finds strips on the LAN, and [`reconnect`] resolves a saved
//! device by its stable id. Stage 1 adds the tray application on top (see
//! `docs/superpowers/specs`).

pub mod ambilight;
pub mod app;
pub mod config;
pub mod diagnostics;
pub mod discovery;
pub mod i18n;
pub mod ipc;
pub mod platform;
pub mod protocol;
pub mod reconnect;
pub mod ui;
