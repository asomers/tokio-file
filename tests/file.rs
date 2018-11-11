extern crate divbuf;
extern crate futures;
extern crate tempdir;
extern crate tokio;
extern crate tokio_file;

use divbuf::DivBufShared;
use futures::future::lazy;
use std::borrow::{Borrow, BorrowMut};
use std::fs;
use std::io::{Read, Write};
use tempdir::TempDir;
use tokio_file::File;
use tokio::runtime::current_thread;

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
    let file = t!(File::open(&path));
    let metadata = file.metadata().unwrap();
    assert_eq!(9000, metadata.len());
}

/// Demonstrate use of `tokio_file::File::new` with user-controlled options.
/// It should fail because the file doesn't already exist and `O_CREAT` is not
/// specified.
#[test]
fn new_nocreat() {
    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("nonexistent");
    let r = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(false)
        .open(&path)
        .map(tokio_file::File::new);
    assert!(r.is_err());
}

#[test]
fn read_at() {
    const WBUF: &[u8] = b"abcdef";
    const EXPECT: &[u8] = b"cdef";
    let dbs = DivBufShared::from(vec![0; 4]);
    let rbuf = Box::new(dbs.try_mut().unwrap());
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at_divbuf_mut");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let r = t!(rt.block_on(lazy(|| {
        file.read_at(rbuf, off).expect("read_at failed early")
    })));
    assert_eq!(r.value.unwrap() as usize, EXPECT.len());

    let mut buf_ref = r.into_buf_ref();
    let borrowed : &mut BorrowMut<[u8]> = buf_ref.boxed_mut_slice()
                                                 .unwrap()
                                                 .borrow_mut();
    assert_eq!(&borrowed.borrow_mut()[..], &EXPECT[..]);
}

#[test]
fn readv_at() {
    const WBUF: &[u8] = b"abcdefghijklmnopqrwtuvwxyz";
    const EXPECT0: &[u8] = b"cdef";
    const EXPECT1: &[u8] = b"ghijklmn";
    let dbs0 = DivBufShared::from(vec![0; 4]);
    let rbuf0 = Box::new(dbs0.try_mut().unwrap());
    let dbs1 = DivBufShared::from(vec![0; 8]);
    let rbuf1 = Box::new(dbs1.try_mut().unwrap());
    let rbufs : Vec<Box<BorrowMut<[u8]>>> = vec![rbuf0, rbuf1];
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("readv_at");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let mut ri = t!(rt.block_on(lazy(|| {
        file.readv_at(rbufs, off).ok().expect("read_at failed early")
    })));

    let mut r0 = ri.next().unwrap();
    assert_eq!(r0.value.unwrap() as usize, EXPECT0.len());
    let b0 : &mut BorrowMut<[u8]> = r0.buf.boxed_mut_slice().unwrap().borrow_mut();
    assert_eq!(&b0.borrow_mut()[..], &EXPECT0[..]);

    let mut r1 = ri.next().unwrap();
    assert_eq!(r1.value.unwrap() as usize, EXPECT1.len());
    let b1 : &mut BorrowMut<[u8]> = r1.buf.boxed_mut_slice().unwrap().borrow_mut();
    assert_eq!(&b1.borrow_mut()[..], &EXPECT1[..]);

    assert!(ri.next().is_none());
}

#[test]
fn sync_all() {
    const WBUF: &[u8] = b"abcdef";

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("sync_all");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let r = t!(rt.block_on(lazy(|| {
        file.sync_all().ok().expect("sync_all failed early")
    })));
    assert!(r.value.is_none());
}

#[test]
fn write_at() {
    let dbs = DivBufShared::from(&b"abcdef"[..]);
    let wbuf = Box::new(dbs.try().unwrap());
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let r = t!(rt.block_on(lazy(|| {
        file.write_at(wbuf.clone(), 0).ok().expect("write_at failed early")
    })));
    assert_eq!(r.value.unwrap() as usize, wbuf.len());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(&wbuf[..], &rbuf[..]);
}

#[test]
fn writev_at() {
    const EXPECT: &[u8] = b"abcdefghij";
    let dbs0 = DivBufShared::from(&b"abcdef"[..]);
    let wbuf0 = Box::new(dbs0.try().unwrap());
    let dbs1 = DivBufShared::from(&b"ghij"[..]);
    let wbuf1 = Box::new(dbs1.try().unwrap());
    let wbufs : Vec<Box<Borrow<[u8]>>> = vec![wbuf0, wbuf1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let mut wi = t!(rt.block_on(lazy(|| {
        file.writev_at(wbufs, 0).ok().expect("writev_at failed early")
    })));

    let w0 = wi.next().unwrap();
    assert_eq!(w0.value.unwrap() as usize, dbs0.len());
    let b0 : &Borrow<[u8]> = w0.buf.boxed_slice().unwrap().borrow();
    assert_eq!(&dbs0.try().unwrap()[..], b0.borrow());

    let w1 = wi.next().unwrap();
    assert_eq!(w1.value.unwrap() as usize, dbs1.len());
    let b1 : &Borrow<[u8]> = w1.buf.boxed_slice().unwrap().borrow();
    assert_eq!(&dbs1.try().unwrap()[..], b1.borrow());

    assert!(wi.next().is_none());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}

#[test]
fn write_at_static() {
    const WBUF: &[u8] = b"abcdef";
    let wbuf = Box::new(WBUF);
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    {
        let file = t!(File::open(&path));
        let mut rt = current_thread::Runtime::new().unwrap();
        let r = t!(rt.block_on(lazy(|| {
            file.write_at(wbuf, 0).ok().expect("write_at failed early")
        })));
        assert_eq!(r.value.unwrap() as usize, WBUF.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}

#[test]
fn writev_at_static() {
    const EXPECT: &[u8] = b"abcdefghi";
    const WBUF0: &[u8] = b"abcdef";
    const WBUF1: &[u8] = b"ghi";
    let wbuf0 = Box::new(WBUF0);
    let wbuf1 = Box::new(WBUF1);
    let wbufs : Vec<Box<Borrow<[u8]>>> = vec![wbuf0, wbuf1];
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("writev_at_static");
    let file = t!(File::open(&path));
    let mut rt = current_thread::Runtime::new().unwrap();
    let mut wi = t!(rt.block_on(lazy(|| {
        file.writev_at(wbufs, 0).ok().expect("writev_at failed early")
    })));

    let w0 = wi.next().unwrap();
    assert_eq!(w0.value.unwrap() as usize, WBUF0.len());

    let w1 = wi.next().unwrap();
    assert_eq!(w1.value.unwrap() as usize, WBUF1.len());

    assert!(wi.next().is_none());

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, EXPECT.len());
    assert_eq!(rbuf, EXPECT);
}
