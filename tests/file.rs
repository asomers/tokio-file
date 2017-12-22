extern crate bytes;
extern crate futures;
extern crate tempdir;
extern crate tokio_core;
extern crate tokio_file;

use bytes::{Bytes, BytesMut};
use std::fs;
use std::io::Read;
use std::io::Write;
use tempdir::TempDir;
use tokio_file::{BufRef, File};
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
    assert_eq!(r.value.unwrap() as usize, EXPECT.len());

    assert_eq!(r.buf.into_bytes_mut().unwrap(), EXPECT);
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
    assert_eq!(r.value.unwrap() as usize, EXPECT.len());
    assert_eq!(r.buf.into_bytes_mut().unwrap(), EXPECT);
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
    let mut ri = t!(l.run(fut));

    let r0 = ri.next().unwrap();
    assert_eq!(r0.value.unwrap() as usize, EXPECT0.len());
    if let BufRef::BytesMut(buf) = r0.buf {
        assert_eq!(buf, EXPECT0);
    } else {
        panic!("Unexpected buffer type");
    }

    let r1 = ri.next().unwrap();
    assert_eq!(r1.value.unwrap() as usize, EXPECT1.len());
    if let BufRef::BytesMut(buf) = r1.buf {
        assert_eq!(buf, EXPECT1);
    } else {
        panic!("Unexpected buffer type");
    }

    assert!(ri.next().is_none());
}

#[test]
fn sync_all() {
    const WBUF: &'static [u8] = b"abcdef";

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("sync_all");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.sync_all().ok().expect("sync_all failed early");
    let r = t!(l.run(fut));
    assert!(r.value.is_none());
}

#[test]
fn write_at() {
    let wbuf = Bytes::from(&b"abcdef"[..]);
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.write_at(wbuf.clone(), 0).ok().expect("write_at failed early");
    let r = t!(l.run(fut));
    assert_eq!(r.value.unwrap() as usize, wbuf.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(rbuf, wbuf);
}

#[test]
fn writev_at() {
    const EXPECT: &'static [u8] = b"abcdefghij";
    let wbuf0 = Bytes::from(&b"abcdef"[..]);
    let wbuf1 = Bytes::from(&b"ghij"[..]);
    let wbufs = [wbuf0, wbuf1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.writev_at(&wbufs, 0).ok().expect("writev_at failed early");
    let mut wi = t!(l.run(fut));

    let w0 = wi.next().unwrap();
    assert_eq!(w0.value.unwrap() as usize, wbufs[0].len());
    if let BufRef::Bytes(buf) = w0.buf {
        assert_eq!(buf, wbufs[0]);
    } else {
        panic!("Unexpected buffer type");
    }

    let w1 = wi.next().unwrap();
    assert_eq!(w1.value.unwrap() as usize, wbufs[1].len());
    if let BufRef::Bytes(buf) = w1.buf {
        assert_eq!(buf, wbufs[1]);
    } else {
        panic!("Unexpected buffer type");
    }

    assert!(wi.next().is_none());

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
        assert_eq!(r.value.unwrap() as usize, WBUF.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}

#[test]
fn writev_at_static() {
    const EXPECT: &'static [u8] = b"abcdefghi";
    const WBUF0: &'static [u8] = b"abcdef";
    const WBUF1: &'static [u8] = b"ghi";
    let wbufs = [WBUF0, WBUF1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at_static");
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let fut = file.writev_at(&wbufs, 0).ok().expect("writev_at failed early");
    let mut wi = t!(l.run(fut));

    let w0 = wi.next().unwrap();
    assert_eq!(w0.value.unwrap() as usize, wbufs[0].len());

    let w1 = wi.next().unwrap();
    assert_eq!(w1.value.unwrap() as usize, wbufs[1].len());

    assert!(wi.next().is_none());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}
