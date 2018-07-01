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
