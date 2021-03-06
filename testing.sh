#!/usr/bin/env bash
# Assumes a file $test_dir that contains folders. The name of these folders is
# the name of the test. Each folder contains a file "args" which is a string fed
# to both find commands. Then output is saved and compared.

# find executable and test directory
test_dir=$(realpath tests)
myfind=$(realpath ./target/release/myfind)
find=$(which gfind)
if [ "$?" -eq 1 ]; then
    find=$(which find)
fi
original_dir=$PWD

all_passed=true
fail () {
    echo -e "\e[31mfailed\e[39m"
    all_passed=false
}
pass () {
    echo -e "\e[32mok\e[39m"
}

# run tests
echo -e "\n\n\t\e[1m\e[32m Start Integration Tests\e[0m\n"
echo -e "Found myfind executable \"$myfind\""
echo -e "Found find executable \"$find\"\n"
for tst in $test_dir/*; do
    if [ -f "$tst/args" ] && [ -f "$tst/prep" ]; then
        cd "$tst" && bash -c "$tst/prep"
    fi
done
for tst in $test_dir/*; do
    if [ -f "$tst/args" ]; then
        cd $tst || exit
        args=$(cat args)
        $myfind $args > test.out 2> test.err
        echo $? > test.exit
        $find $args  > crct.out 2> crct.err
        echo $? > crct.exit
        echo -n "Testing $(basename $tst) with args: $args ... "
        diff test.err crct.err && \
            diff test.out crct.out && \
            diff test.exit crct.exit && \
            pass || fail
        cd $original_dir
    fi
done

if [ "$all_passed" = true ]; then
    color="\e[32m"
else
    color="\e[31m"
fi
echo -e "\n\t$color\e[1mFinished Integration Tests\e[39m\n"

