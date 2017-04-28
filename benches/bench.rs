#![feature(test)]

extern crate futures;
extern crate tempdir;
extern crate tokio_core;
extern crate tokio_file;
extern crate test;

use futures::sync::oneshot;
use std::fs;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use tempdir::TempDir;
use test::Bencher;
use tokio_core::reactor::Core;
use tokio_file::file::File;

const FLEN: usize = 4096;

#[bench]
fn bench_aio_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new("tokio-file").unwrap();
    let path = dir.path().join("aio_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    let rbuf = Rc::new(vec![0; FLEN].into_boxed_slice());
    f.write(&wbuf).expect("write failed");

    // Prep the reactor
    let mut l = Core::new().unwrap();
    let file = File::open(path, l.handle().clone()).unwrap();

    bench.iter(move || {
        let fut = file.read_at(rbuf.clone(), 0).ok().expect("read_at failed early");
        assert_eq!(l.run(fut).unwrap() as usize, wbuf.len());
    })
}

// For comparison, benchmark the equivalent operation using threads
// instead of POSIX AIO
#[bench]
fn bench_threaded_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new("tokio-file").unwrap();
    let path = dir.path().join("threaded_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    let rbuf = Arc::new(Mutex::new(vec![0; FLEN]));
    f.write(&wbuf).expect("write failed");

    // Prep the reactor
    let mut l = Core::new().unwrap();
    let pathclone = path.clone();
    let file = Arc::new(Mutex::new(fs::File::open(&pathclone).unwrap()));

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let rclone = rbuf.clone();
        let fclone = file.clone();
        thread::spawn(move || {
            let mut d = rclone.lock().unwrap();
            let len = fclone.lock().unwrap().read_at(&mut d, 0).expect("read failed");
            tx.send(len).expect("sending failed");
        });
        assert_eq!(l.run(rx).expect("receiving failed") as usize, FLEN);
    });
}
    


