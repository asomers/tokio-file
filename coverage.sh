#! /bin/sh -e
#
# Generate a code coverage report
#
# Requirements:
# sudo pkg install grcov
# cargo install grcov
# rustup component add llvm-tools-preview
#
# Usage:
# tools/coverage.sh

TOOLCHAIN=nightly

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
	--ignore "benches/*" \
	--ignore "examples/*" \
	-o ./coverage/ \
	.
