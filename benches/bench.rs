#![feature(test)]

extern crate test;

use divbuf::{DivBufMut, DivBufShared};
use futures::sync::oneshot;
use std::fs;
use std::io::Write;
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use tempfile::TempDir;
use test::Bencher;
use tokio::runtime::current_thread::Runtime;
use tokio_file::File;

const FLEN: usize = 1<<19;

#[bench]
fn bench_aio_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("aio_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    let dbs = DivBufShared::from(vec![0; FLEN]);
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let mut runtime = Runtime::new().unwrap();
    let file = File::open(path).unwrap();

    bench.iter(move || {
        let rbuf = Box::new(dbs.try_mut().unwrap());
        let fut = file.read_at(rbuf, 0)
            .expect("read_at failed early");
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
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("threaded_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    let dbs = DivBufShared::from(vec![0; FLEN]);
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let mut runtime = Runtime::new().unwrap();
    let file = Arc::new(Mutex::new(fs::File::open(&path).unwrap()));

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let mut rbuf = dbs.try_mut().unwrap();
        let fclone = file.clone();
        thread::spawn(move || {
            let len = fclone.lock()
                            .unwrap()
                            .read_at(&mut rbuf[..], 0)
                            .expect("read failed");
            drop(rbuf);
            tx.send(len).expect("sending failed");
        });
        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len as usize, FLEN);
    });
}
    
struct TpOpspec {
    f:  Arc<Mutex<fs::File>>,
    buf: DivBufMut,
    offset: u64,
    tx: oneshot::Sender<usize>
}

// For comparison, benchmark the equivalent operation using a single thread to
// simulate a thread pool
#[bench]
fn bench_threadpool_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("threadpool_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the thread "pool"
    let (_ptx, prx) = mpsc::channel();
    let ptx = Arc::new(_ptx);
    thread::spawn(move || {
        loop {
            let v: Option<TpOpspec> = prx.recv().unwrap();
            if let Some(mut op) = v {
                let f = op.f.lock().unwrap();
                let len = {
                    let sl = &mut op.buf[..];
                    f.read_at(sl, op.offset).unwrap()
                };
                drop(op.buf);
                op.tx.send(len).expect("send failed");
            } else {
                break;
            }
        }
    });

    // Prep the reactor
    let mut runtime = Runtime::new().unwrap();
    let file = Arc::new(Mutex::new(fs::File::open(&path).unwrap()));
    let ptxclone = ptx.clone();

    bench.iter(move || {
        let dbs = DivBufShared::from(vec![0; FLEN]);
        let (tx, rx) = oneshot::channel::<usize>();
        let opspec = TpOpspec {
            f: file.clone(),
            buf: dbs.try_mut().unwrap(),
            offset: 0,
            tx
        };
        ptxclone.send(Some(opspec)).unwrap();

        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len as usize, FLEN);
    });
    ptx.send(None).unwrap();
}
