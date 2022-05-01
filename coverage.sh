#! /bin/sh -e
#
# Generate a code coverage report
#
# Requirements:
# sudo pkg install fio grcov
# cargo install grcov
# rustup component add llvm-tools-preview
#
# Usage:
# tools/coverage.sh

# Must lock toolchain to to Rust bug 94616
# https://github.com/rust-lang/rust/issues/94616
TOOLCHAIN=nightly-2022-02-15

export LLVM_PROFILE_FILE="tokio-file-%p-%m.profraw"
export RUSTFLAGS="-C instrument-coverage"
cargo +$TOOLCHAIN build --all-targets
# Some tests require root privileges
sudo -E cargo +$TOOLCHAIN test

rustup run $TOOLCHAIN grcov \
	--binary-path $PWD/target/debug \
	-s . \
	-t html \
	--branch \
	--ignore-not-existing \
	--excl-line LCOV_EXCL_LINE \
	--excl-start LCOV_EXCL_START \
	--excl-stop LCOV_EXCL_STOP \
	--ignore "tests/*" \
	--ignore "examples/*" \
	-o ./coverage/ \
	.
