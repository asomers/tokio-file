#![feature(test)]

extern crate test;

use futures::channel::oneshot;
use std::io::{SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use tempfile::TempDir;
use test::Bencher;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::runtime::{self, Runtime};
use tokio_file::File;

const FLEN: usize = 1<<19;

fn runtime() -> Runtime {
    runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .unwrap()
}

#[bench]
fn bench_aio_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("aio_read");
    let mut f = std::fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let runtime = runtime();
    let file = File::open(path).unwrap();

    let mut rbuf = vec![0u8; FLEN];
    bench.iter(move || {
        let len = runtime.block_on(async{
            file.read_at(&mut rbuf[..], 0)
                .expect("read_at failed early").await
        }).unwrap().value.unwrap();
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
    let mut f = std::fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let runtime = runtime();
    let file = Arc::new(Mutex::new(std::fs::File::open(&path).unwrap()));

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let fclone = file.clone();
        thread::spawn(move || {
            let mut rbuf = vec![0u8; FLEN];
            let len = fclone.lock()
                            .unwrap()
                            .read_at(&mut rbuf[..], 0)
                            .expect("read failed");
            tx.send(len).expect("sending failed");
        });
        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len as usize, FLEN);
    });
}
    
struct TpOpspec {
    f:  Arc<Mutex<std::fs::File>>,
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
    let mut f = std::fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the thread "pool"
    let (_ptx, prx) = mpsc::channel();
    let ptx = Arc::new(_ptx);
    thread::spawn(move || {
        loop {
            let v: Option<TpOpspec> = prx.recv().unwrap();
            if let Some(op) = v {
                let mut rbuf = vec![0; FLEN];
                let f = op.f.lock().unwrap();
                let len = {
                    f.read_at(&mut rbuf[..], op.offset).unwrap()
                };
                op.tx.send(len).expect("send failed");
            } else {
                break;
            }
        }
    });

    // Prep the reactor
    let runtime = runtime();
    let file = Arc::new(Mutex::new(std::fs::File::open(&path).unwrap()));
    let ptxclone = ptx.clone();

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let opspec = TpOpspec {
            f: file.clone(),
            offset: 0,
            tx
        };
        ptxclone.send(Some(opspec)).unwrap();

        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len as usize, FLEN);
    });
    ptx.send(None).unwrap();
}

// Use Tokio's builtin thread-based file system routines.
#[bench]
fn bench_tokio_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("aio_read");
    let mut f = std::fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let runtime = runtime();
    let mut file = runtime.block_on(tokio::fs::File::open(path)).unwrap();

    let mut rbuf = vec![0u8; FLEN];
    bench.iter(move || {
        let len = runtime.block_on(async{
            file.seek(SeekFrom::Start(0)).await.unwrap();
            file.read_exact(&mut rbuf[..]).await
        }).unwrap();
        assert_eq!(len as usize, wbuf.len());
    })
}
