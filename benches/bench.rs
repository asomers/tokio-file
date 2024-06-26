#![feature(test)]

extern crate test;

use std::{
    fs,
    io::{SeekFrom, Write},
    sync::{mpsc, Arc, Mutex},
    thread,
};

use futures::channel::oneshot;
use tempfile::TempDir;
use test::Bencher;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    runtime::{self, Runtime},
};

const FLEN: usize = 1 << 19;

fn runtime() -> Runtime {
    runtime::Builder::new_multi_thread()
        .enable_io()
        .build()
        .unwrap()
}

#[bench]
fn bench_aio_read(bench: &mut Bencher) {
    use tokio_file::AioFileExt;

    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("aio_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let runtime = runtime();
    let file = fs::File::open(path).unwrap();

    let mut rbuf = vec![0u8; FLEN];
    bench.iter(|| {
        let len = runtime
            .block_on(async {
                file.read_at(&mut rbuf[..], 0)
                    .expect("read_at failed early")
                    .await
            })
            .unwrap();
        assert_eq!(len, wbuf.len());
    });
}

// For comparison, benchmark the equivalent operation using threads
// instead of POSIX AIO
#[bench]
fn bench_threaded_read(bench: &mut Bencher) {
    use std::os::unix::fs::FileExt;

    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("threaded_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let runtime = runtime();
    let file = Arc::new(Mutex::new(fs::File::open(&path).unwrap()));

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let fclone = file.clone();
        thread::spawn(move || {
            let mut rbuf = vec![0u8; FLEN];
            let len = fclone
                .lock()
                .unwrap()
                .read_at(&mut rbuf[..], 0)
                .expect("read failed");
            tx.send(len).expect("sending failed");
        });
        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len, FLEN);
    });
}

struct TpOpspec {
    f:      Arc<Mutex<fs::File>>,
    offset: u64,
    tx:     oneshot::Sender<usize>,
}

// For comparison, benchmark the equivalent operation using a single thread to
// simulate a thread pool
#[bench]
fn bench_threadpool_read(bench: &mut Bencher) {
    use std::os::unix::fs::FileExt;

    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("threadpool_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the thread "pool"
    let (_ptx, prx) = mpsc::channel();
    let ptx = Arc::new(_ptx);
    thread::spawn(move || loop {
        let v: Option<TpOpspec> = prx.recv().unwrap();
        if let Some(op) = v {
            let mut rbuf = vec![0; FLEN];
            let f = op.f.lock().unwrap();
            let len = { f.read_at(&mut rbuf[..], op.offset).unwrap() };
            op.tx.send(len).expect("send failed");
        } else {
            break;
        }
    });

    // Prep the reactor
    let runtime = runtime();
    let file = Arc::new(Mutex::new(fs::File::open(&path).unwrap()));
    let ptxclone = ptx.clone();

    bench.iter(move || {
        let (tx, rx) = oneshot::channel::<usize>();
        let opspec = TpOpspec {
            f: file.clone(),
            offset: 0,
            tx,
        };
        ptxclone.send(Some(opspec)).unwrap();

        let len = runtime.block_on(rx).expect("receiving failed");
        assert_eq!(len, FLEN);
    });
    ptx.send(None).unwrap();
}

// Use Tokio's builtin thread-based file system routines.
#[bench]
fn bench_tokio_read(bench: &mut Bencher) {
    // First prep the test file
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("aio_read");
    let mut f = fs::File::create(&path).unwrap();
    let wbuf = vec![0; FLEN];
    f.write_all(&wbuf).expect("write failed");

    // Prep the reactor
    let runtime = runtime();
    let mut file = runtime.block_on(tokio::fs::File::open(path)).unwrap();

    let mut rbuf = vec![0u8; FLEN];
    bench.iter(move || {
        let len = runtime
            .block_on(async {
                file.seek(SeekFrom::Start(0)).await.unwrap();
                file.read_exact(&mut rbuf[..]).await
            })
            .unwrap();
        assert_eq!(len, wbuf.len());
    })
}
