//! Tokio module for file I/O

extern crate futures;
extern crate libc;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tokio_core;

pub mod file;

pub use file::{AioReadFut, AioSyncFut, AioWriteFut, File, WriteAtable};
