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
//! use std::fs;
//! use std::io::Read;
//! use tempfile::TempDir;
//! use tokio::runtime;
//!
//! let contents = b"abcdef";
//! let mut rbuf = Vec::new();
//!
//! let dir = TempDir::new().unwrap();
//! let path = dir.path().join("foo");
//! let file = fs::OpenOptions::new()
//!     .create(true)
//!     .write(true)
//!     .open(&path)
//!     .map(tokio_file::File::new)
//!     .unwrap();
//! let rt = runtime::Builder::new_current_thread()
//!     .enable_io()
//!     .build()
//!     .unwrap();
//! let r = rt.block_on(async {
//!     file.write_at(contents, 0).unwrap().await
//! }).unwrap();
//! assert_eq!(r, contents.len());
//! drop(file);
//!
//! let mut file = fs::File::open(&path).unwrap();
//! assert_eq!(file.read_to_end(&mut rbuf).unwrap(), contents.len());
//! assert_eq!(&contents[..], &rbuf[..]);
//! ```
#![deny(missing_docs)]

mod file;

pub use file::{File, ReadAt, ReadvAt, SyncAll, WriteAt, WritevAt};
