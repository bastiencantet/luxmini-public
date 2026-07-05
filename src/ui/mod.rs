//! `AppKit` menu-bar UI: status-bar item, dropdown menu, and the Obj-C action handler.

pub mod handler;
mod menu;
pub mod settings;
mod tray;

pub use menu::build_app;
