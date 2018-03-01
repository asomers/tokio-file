//! Tokio module for file I/O

extern crate divbuf;
extern crate futures;
extern crate libc;
extern crate mio;
extern crate mio_aio;
extern crate nix;
extern crate tokio;

mod file;

pub use file::{AioFut, BufRef, File, WriteAtable};
