extern crate bytes;
extern crate futures;
extern crate tempdir;
extern crate tokio_core;
extern crate tokio_file;

use bytes::{Bytes, BytesMut};
use std::fs;
use std::io::Read;
use std::io::Write;
use std::ops::Deref;
use tempdir::TempDir;
use tokio_file::File;
use tokio_core::reactor::Core;

macro_rules! t {
    ($e:expr) => (match $e {
        Ok(e) => e,
        Err(e) => panic!("{} failed with {:?}", stringify!($e), e),
    })
}

#[test]
fn metadata() {
    let wbuf = vec![0; 9000].into_boxed_slice();
    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at");
    let mut f = t!(fs::File::create(&path));
    f.write(&wbuf).expect("write failed");
    let l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let metadata = file.metadata().unwrap();
    assert_eq!(9000, metadata.len());
}

#[test]
fn read_at() {
    const WBUF: &'static [u8] = b"abcdef";
    const EXPECT: &'static [u8] = b"cdef";
    let rbuf = BytesMut::from(vec![0; 4]);
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at_bytes_mut");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.read_at(rbuf, off).ok().expect("read_at failed early");
    let r = t!(l.run(fut));
    assert_eq!(r.value as usize, EXPECT.len());

    assert_eq!(r.buf.unwrap().deref().deref(), EXPECT);
}

#[test]
fn read_at_long() {
    const WBUF: &'static [u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    const EXPECT: &'static [u8] = b"cdefghijklmnopqrstuvwxyz0123456789";
    let rbuf = BytesMut::from(vec![0; 34]);
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at_bytes_mut");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.read_at(rbuf, off).ok().expect("read_at failed early");
    let r = t!(l.run(fut));
    assert_eq!(r.value as usize, EXPECT.len());

    //println!("rbuf p={:?} => {:?}", rbuf.as_ptr(), unsafe{*rbuf.as_ptr()});
    assert_eq!(r.buf.unwrap().deref().deref(), EXPECT);
}

#[test]
fn readv_at() {
    const WBUF: &'static [u8] = b"abcdefghijklmnopqrwtuvwxyz";
    const EXPECT0: &'static [u8] = b"cdef";
    const EXPECT1: &'static [u8] = b"ghijklmn";
    let rbuf0 = BytesMut::from(vec![0; 4]);
    let rbuf1 = BytesMut::from(vec![0; 8]);
    let rbufs = vec![rbuf0, rbuf1];
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("readv_at");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.readv_at(rbufs, off).ok().expect("read_at failed early");
    let r = t!(l.run(fut));
    assert_eq!(r.value as usize, EXPECT0.len() + EXPECT1.len());

    // TODO: fix readv_at's buffer handling
    assert_eq!(r.buf.unwrap().deref().deref(), EXPECT0);
    //assert_eq!(r.buf.unwrap().deref().deref(), EXPECT1);
}

#[test]
fn sync_all() {
    const WBUF: &'static [u8] = b"abcdef";

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("sync_all");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.sync_all().ok().expect("sync_all failed early");
        let r = t!(l.run(fut));
        assert_eq!(r.value, ());
    }
}

#[test]
fn write_at() {
    let wbuf = Bytes::from(&b"abcdef"[..]);
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.write_at(wbuf.clone(), 0).ok().expect("write_at failed early");
        let r = t!(l.run(fut));
        assert_eq!(r.value as usize, wbuf.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(rbuf, wbuf.deref().deref());
}

#[test]
fn writev_at() {
    const EXPECT: &'static [u8] = b"abcdefghij";
    let wbuf0 = Bytes::from(&b"abcdef"[..]);
    let wbuf1 = Bytes::from(&b"ghij"[..]);
    let total_len = wbuf0.len() + wbuf1.len();
    let wbufs = [wbuf0, wbuf1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.writev_at(&wbufs, 0).ok().expect("writev_at failed early");
        let r = t!(l.run(fut));
        assert_eq!(r.value as usize, total_len);
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}

#[test]
fn write_at_static() {
    const WBUF: &'static [u8] = b"abcdef";
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.write_at(WBUF, 0).ok().expect("write_at failed early");
        let r = t!(l.run(fut));
        assert_eq!(r.value as usize, WBUF.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}

#[test]
fn write_at_bytes() {
    let wbuf = Bytes::from(&"abcdef"[..]);
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.write_at(wbuf.clone(), 0).ok().expect("write_at failed early");
    let r = t!(l.run(fut));
    assert_eq!(r.value as usize, wbuf.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(rbuf, wbuf);
}

#[test]
fn writev_at_static() {
    const EXPECT: &'static [u8] = b"abcdefghi";
    const WBUF0: &'static [u8] = b"abcdef";
    const WBUF1: &'static [u8] = b"ghi";
    let total_len = WBUF0.len() + WBUF1.len();
    let wbufs = [WBUF0, WBUF1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at_static");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.writev_at(&wbufs, 0).ok().expect("writev_at failed early");
        let r = t!(l.run(fut));
        assert_eq!(r.value as usize, total_len);
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}
