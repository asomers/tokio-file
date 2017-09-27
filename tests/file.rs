extern crate futures;
extern crate tempdir;
extern crate tokio_core;
extern crate tokio_file;

use std::fs;
use std::io::Read;
use std::io::Write;
use std::ops::Deref;
use std::rc::Rc;
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
    let mut l = t!(Core::new());
    let file = t!(File::open(&path, l.handle()));
    let metadata = file.metadata().unwrap();
    assert_eq!(9000, metadata.len());
}

#[test]
fn read_at() {
    const WBUF: &'static [u8] = b"abcdef";
    const EXPECT: &'static [u8] = b"cdef";
    let rbuf = Rc::new(vec![0; 4].into_boxed_slice());
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.read_at(rbuf.clone(), off).ok().expect("read_at failed early");
        assert_eq!(t!(l.run(fut)) as usize, EXPECT.len());
    }

    assert_eq!(rbuf.deref().deref(), EXPECT);
}

#[test]
fn readv_at() {
    const WBUF: &'static [u8] = b"abcdefghijklmnopqrwtuvwxyz";
    const EXPECT0: &'static [u8] = b"cdef";
    const EXPECT1: &'static [u8] = b"ghijklmn";
    let rbuf0 = Rc::new(vec![0; 4].into_boxed_slice());
    let rbuf1 = Rc::new(vec![0; 8].into_boxed_slice());
    let rbufs = [rbuf0.clone(), rbuf1.clone()];
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("readv_at");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.readv_at(&rbufs, off).ok().expect("read_at failed early");
        assert_eq!(t!(l.run(fut)) as usize, EXPECT0.len() + EXPECT1.len());
    }

    assert_eq!(rbuf0.deref().deref(), EXPECT0);
    assert_eq!(rbuf1.deref().deref(), EXPECT1);
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
        assert_eq!(t!(l.run(fut)), ());
    }
}

#[test]
fn write_at() {
    let wbuf = Rc::new(String::from("abcdef").into_bytes().into_boxed_slice());
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.write_at(wbuf.clone(), 0).ok().expect("write_at failed early");
        assert_eq!(t!(l.run(fut)) as usize, wbuf.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, wbuf.len());
    assert_eq!(rbuf, wbuf.deref().deref());
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
        assert_eq!(t!(l.run(fut)) as usize, WBUF.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}
