# find

This is a program that implements `find` identically to the standard unix
command.

## Subcommands

- `name` pattern (including wildcards)
- `mtime n` (to simplify, I will only test with n=0, so don't bother with negatives or plus sign).
- `type t` where `t` is
  - `b` a block device
  - `c` character special
  - `d` directory
  - `p` named pipe (FIFO)
  - `f` regular file
  - `l` symbolic link
  - `s` socket
- `exec command` (only the ; variant).
- `print`
- `L` (follow symbolic links)

## Notes

- We assume that `.` is chosen to start the search if no other directory is given.
- Output should be exactly the same as the `find` command for linux. This can be
  script testable.

## Progress

- [x] name
- [x] mtime
- [x] type
- [x] exec
- [x] print
- [x] L
- [x] Parsing cli correctly
- [x] error messages are definitely different.
- [ ] Testing
  - [x] framework up
  - [ ] mtime
  - [x] type
  - [x] name
  - [x] exec
  - [x] print
  - [x] L
