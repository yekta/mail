//! The client core: the local copy of the mail, the link to the server and everything the apps
//! draw. The apps link it as a static library (`ffi.rs`: JSON commands in, JSON events out).

pub mod api;
pub mod core;
mod demo;
pub mod ffi;
mod http;
mod link;
pub mod render;
pub mod store;
