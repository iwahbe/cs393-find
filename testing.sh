#!/usr/bin/env bash

cargo test
cargo build --release

test_dir=tests
find=$(realpath ./target/release/find)
original_dir=$PWD

for tst in $test_dir/*; do
    cd $tst
    args=$(cat args)
    $find $args > test.out 2> test.err
    find $args  > crct.out 2> crct.err
    diff test.out crct.out
    diff test.err crct.err
done

cd $original_dir
