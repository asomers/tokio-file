#![feature(test)]

extern crate divbuf;
extern crate futures;
extern crate tempdir;
extern crate tokio;
extern crate tokio_file;
extern crate test;

use divbuf::DivBufShared;
use futures::sync::oneshot;
use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::vec;
use tempdir::TempDir;
use test::Bencher;
use tokio::reactor::Handle;
use tokio::runtime::current_thread::Runtime;
use tokio_file::File;

const FLEN: usize = 4096;

#[bench]
fn bench_aio_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new("tokio-file").unwrap();
    let path = dir.path().join("aio_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    let dbs = DivBufShared::from(vec![0; FLEN]);
    f.write(&wbuf).expect("write failed");

    // Prep the reactor
    let mut runtime = Runtime::new().unwrap();
    let file = File::open(path, Handle::current()).unwrap();

    bench.iter(move || {
        let rbuf = Box::new(dbs.try_mut().unwrap());
        let fut = file.read_at(rbuf, 0).ok().expect("read_at failed early");
        let len = runtime.block_on(fut)
                         .unwrap()
                         .value.unwrap();
        assert_eq!(len as usize, wbuf.len());
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
    let mut runtime = Runtime::new().unwrap();
    let pathclone = path.clone();
    let file = Arc::new(Mutex::new(fs::File::open(&pathclone).unwrap()));

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let rclone = rbuf.clone();
        let fclone = file.clone();
        thread::spawn(move || {
            let mut d = rclone.lock().unwrap();
            let len = fclone.lock()
                            .unwrap()
                            .read_at(&mut d, 0)
                            .expect("read failed");
            tx.send(len).expect("sending failed");
        });
        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len as usize, FLEN);
    });
}
    
struct TpOpspec {
    f:  Arc<Mutex<fs::File>>,
    buf: RefCell<vec::Vec<u8>>,
    offset: u64,
    tx: oneshot::Sender<usize>
}

// For comparison, benchmark the equivalent operation using a single thread to
// simulate a thread pool
#[bench]
fn bench_threadpool_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new("tokio-file").unwrap();
    let path = dir.path().join("threadpool_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write(&wbuf).expect("write failed");

    // Prep the thread "pool"
    let (_ptx, prx) = mpsc::channel();
    let ptx = Arc::new(_ptx);
    thread::spawn(move || {
        loop {
            let v: Option<TpOpspec> = prx.recv().unwrap();
            if let Some(op) = v {
                let mut buf = op.buf.borrow_mut();
                let f = op.f.lock().unwrap();
                let len: usize = f.read_at(&mut buf, op.offset).unwrap();
                op.tx.send(len).expect("send failed");
            } else {
                break;
            }
        }
    });

    // Prep the reactor
    let mut runtime = Runtime::new().unwrap();
    let pathclone = path.clone();
    let file = Arc::new(Mutex::new(fs::File::open(&pathclone).unwrap()));
    let ptxclone = ptx.clone();

    bench.iter(move || {
        let rbuf = vec![0; FLEN];
        let (tx, rx) = oneshot::channel::<usize>();
        let opspec = TpOpspec{
            f: file.clone(),
            buf: RefCell::new(rbuf),
            offset: 0,
            tx: tx};
        ptxclone.send(Some(opspec)).unwrap();

        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len as usize, FLEN);
    });
    ptx.send(None).unwrap();
}
