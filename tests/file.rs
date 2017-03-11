extern crate futures;
extern crate tempdir;
extern crate tokio_core;
extern crate tokio_file;

use std::fs;
use std::io::Read;
use std::io::Write;
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
fn read_at() {
    const WBUF: &'static [u8] = b"abcdef";
    const EXPECT: &'static [u8] = b"cdef";
    let mut rbuf = vec![0;4];
    let off = 2;

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("read_at");
    let mut f = t!(fs::File::create(&path));
    f.write(WBUF).expect("write failed");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.read_at(&mut rbuf, off).ok().expect("read_at failed early");
        assert_eq!(t!(l.run(fut)) as usize, EXPECT.len());
    }

    assert_eq!(rbuf, EXPECT);
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
    const WBUF: &'static [u8] = b"abcdef";
    let mut rbuf = Vec::new();

    let dir = t!(TempDir::new("tokio-file"));
    let path = dir.path().join("write_at");
    {
        let mut l = t!(Core::new());
        let file = t!(File::open(&path, l.handle()));
        let fut = file.write_at(&WBUF, 0).ok().expect("write_at failed early");
        assert_eq!(t!(l.run(fut)) as usize, WBUF.len());
    }

    let mut f = t!(fs::File::open(&path));
    let len = t!(f.read_to_end(&mut rbuf));
    assert_eq!(len, WBUF.len());
    assert_eq!(rbuf, WBUF);
}
