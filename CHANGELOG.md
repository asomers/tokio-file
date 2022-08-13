## [Unreleased] - ReleaseDate

### Changed

- Rewrite `readv_at` and `writev_at` in terms of `aio_readv` and `aio_writev`
  respectively, eliminating the old `lio_listio` based implementation.  This
  means they won't work on less than FreeBSD 13.0.  The native implementations
  are faster.  They're also atomic, meaning that no other process will see part
  of the `writev_at` operation as complete but not all of it, and similarly for
  `readv_at`.  They now take as arguments slices of `IoSliceMut` and `IoSlice`,
  respectively, for better familiarity with the standard library.

  The return type of all futures is now either `usize` or `()` instead of the
  old `AioResult` enum.

  Finally, each asynchronous method now returns a distinct `Future` type,
  rather than all of them returning an `AioFut`.

  Also, `tokio-file` now generates less memory allocator activity, thanks to
  matching changes in its dependencies.
  (#[34](https://github.com/asomers/tokio-file/pull/34))

- Updated Nix to 0.25.0
  (#[35](https://github.com/asomers/tokio-file/pull/35))

### Removed

- `readv_at` and `writev_at` now require at least FreeBSD 13.0, and they no
  repack buffers to meet device alignment requirements.
  (#[34](https://github.com/asomers/tokio-file/pull/34))

## [0.8.0] - 2022-04-21

### Changed

- Updated the MSRV to 1.49.0, because Tokio did.
  (#[30](https://github.com/asomers/tokio-file/pull/30))

- Updated Nix to 0.24.0.  This raises the MSRV to 1.46.0
  (#[31](https://github.com/asomers/tokio-file/pull/31))

## [0.7.0] - 2021-12-11

### Changed

- Updated Nix to 0.22.0.  This changes tokio-file's error types, because we
  reexport from Nix.
  (#[21](https://github.com/asomers/tokio-file/pull/21))

## Fixed

- Fixed `lio_listio` resubmission with Tokio 1.13.0 and later.
  (#[28](https://github.com/asomers/tokio-file/pull/28))

## [0.6.0] - 2021-05-31
### Changed
- Updated to Future 0.3, `std::future`, and Tokio 0.2.  Now tokio-file's
  futures can be used with async/await.  All methods now use borrowed buffers
  rather than owned buffers.
  (#[15](https://github.com/asomers/tokio-file/pull/15))

## [0.5.2] - 2020-02-13
### Fixed
- Fixed `readv_at` on device nodes with unaligned buffers
  (#[13](https://github.com/asomers/tokio-file/pull/13))


## [0.5.1] - 2019-09-05

### Changed
- Updated mio-aio and nix dependencies
  ([49fb7a60](https://github.com/asomers/tokio-file/commit/49fb7a6044cf6954d228b9f4b9497845741b6258))
  ([07502b84](https://github.com/asomers/tokio-file/commit/07502b84c38039c22741395211a7e0a722a6fb52))

## [0.5.0] - 2018-11-29
### Added
- `File` now implements `AsRawFd`.
- `LioFut`'s Item type is now `LioResult`, which indicates which operations
  passed and which failed.
- `File::len` gets the file length, whether a regular file or device node
- `File::{readv_at, writev_at}` now work better with device nodes.

### Removed
- `LioFut` no longer implements `Debug`.

## [0.4.0] - 2018-11-11

### Added
- `File::new` allows creating a tokio file object from an arbitrary
  `std::fs::File`.

### Changed
- `open` no longer takes a `Handle` argument.

## [0.3.0] - 2018-07-01
### Added
- Added `File::readv_at` and `File::writev_at`
- Added `File::metadata`
- Implement `Debug` for `File`.

### Changed
- `read_at` and `write_at` no longer take buffers as `Rc<Box<u8]>>`.  Instead,
  they use `Box<Borrow<[u8]>>` or `Box<BorrowMut<[u8]>>`.
- `read_at`, `write_at`, and `sync_all` now return the same `Future` type.
  It's result is an `AioResult`, which includes the original buffer as well as
  the underlying AIO function's return value.
- Switch from using `tokio-core` to the newer `tokio`.
- Raise minimum version of `mio` to 0.6.13.

### Fixed
- Don't ignore errors from `aio_read`, `aio_write`, and `aio_fsync`.

### Removed
