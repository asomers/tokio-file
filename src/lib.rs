// vim: tw=80
//! Asynchronous File I/O module for Tokio
//!
//! This module provides methods for asynchronous file I/O.  On BSD-based
//! operating systems, it uses mio-aio.  On Linux, it could use libaio, but that
//! isn't implemented yet.
//!
//! # Examples
//!
//! ```
//! # extern crate tempdir;
//! # extern crate tokio;
//! # extern crate tokio_file;
//! use std::borrow::Borrow;
//! use std::fs;
//! use std::io::Read;
//! use tempdir::TempDir;
//! use tokio::runtime::current_thread;
//! use tokio_file;
//!
//! let contents = b"abcdef";
//! let wbuf: Box<Borrow<[u8]>> = Box::new(&contents[..]);
//! let mut rbuf = Vec::new();
//!
//! let dir = TempDir::new("tokio-file").unwrap();
//! let path = dir.path().join("foo");
//! let file = fs::OpenOptions::new()
//!     .create(true)
//!     .write(true)
//!     .open(&path)
//!     .map(tokio_file::File::new)
//!     .unwrap();
//! let mut rt = current_thread::Runtime::new().unwrap();
//! let r = rt.block_on(
//!     file.write_at(wbuf, 0).unwrap()
//! ).unwrap();
//! assert_eq!(r.value.unwrap() as usize, contents.len());
//! drop(file);
//!
//! let mut file = fs::File::open(&path).unwrap();
//! assert_eq!(file.read_to_end(&mut rbuf).unwrap(), contents.len());
//! assert_eq!(&contents[..], &rbuf[..]);
//! ```

extern crate futures;
extern crate mio;
extern crate mio_aio;
#[macro_use] extern crate nix;
extern crate tokio_reactor;

mod file;

pub use file::{AioFut, AioResult, BufRef, File, LioFut, LioResult};
