#!/usr/bin/env bash

cargo test || exit
cargo build --release || exit

test_dir=tests
find=$(realpath ./target/release/find)
original_dir=$PWD

echo -e "\nIntegration Tests"
for tst in $test_dir/*; do
    cd $tst || exit
    args=$(cat args)
    $find $args > test.out 2> test.err
    find $args  > crct.out 2> crct.err
    echo -n "Testing $tst with arg: $args ... "
    diff test.out crct.out > /dev/null && \
        diff test.err crct.err > /dev/null && \
        echo -e "\e[32mok\e[39m" || \
        echo -e "\e[31mfailed\e[39m"
    cd $original_dir
done

cd $original_dir
