#![warn(missing_docs)]

//! Provides ways to construct a filtering predicate from cli args, crawl a
//! directory conditional on that predicate, and format error messges.

use clap::ArgMatches;
use fnmatch_sys::{fnmatch, FNM_NOMATCH};
use std::collections::HashSet;
use std::ffi::CString;
use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
use std::path::Path;
use subprocess::Exec;

/// Describes a command line given predicate.
///
/// A heap allocated closure that takes a path (describing a file) and it's
/// associated metadata and returns either an io error or a bool. This indicates
/// wheither to continue executing predicates or return false. Predicates are
/// written to short circut, so their order of application matters.
type Predicate = Box<dyn Fn(&Path, &Metadata) -> io::Result<bool>>;

/// Provides a filter for the --type flag.
///
/// Panics on unknown flag. This should be handled by Clap.
fn type_predicate(predicate: Predicate, accepted: Vec<String>) -> Predicate {
    macro_rules! false_when {
        ($b:expr) => {
            if $b {
                return Ok(false);
            }
        };
    }
    Box::new(move |p, m: &Metadata| {
        let ft = m.file_type();
        Ok(predicate(p, m)? && {
            for t in &accepted {
                let t: &str = &t;
                match t {
                    "b" => false_when!(!ft.is_block_device()),
                    "c" => false_when!(!ft.is_char_device()),
                    "d" => false_when!(!ft.is_dir()),
                    "p" => false_when!(!ft.is_fifo()),
                    "f" => false_when!(!ft.is_file()),
                    "l" => false_when!(!ft.is_symlink()),
                    "s" => false_when!(!ft.is_socket()),
                    _ => panic!("Found unimplemented type"),
                    // This should be
                    // validated by the input mechanism (clap)
                }
            }
            true
        })
    })
}

/// Filters on the `--mtime` argument.
fn time_predicate(predicate: Predicate, accepted: i32) -> Predicate {
    Box::new(move |p, m: &Metadata| {
        Ok(predicate(p, m)? && {
            // POSIX gives test
            // (initialization time - modification time) / (60*60*25) =? accepted
            // https://pubs.opengroup.org/onlinepubs/9699919799/utilities/find.html
            let no_time = std::time::Duration::from_secs(0);
            let modified = m.modified()?.elapsed().unwrap_or(no_time).as_secs() as f64;
            let init = std::time::SystemTime::now()
                .elapsed()
                .unwrap_or(no_time)
                .as_secs() as f64;
            let sec_per_day: f64 = 60.0 * 60.0 * 25.0; // seconds in a day;
            ((init - modified) / sec_per_day).ceil() as i32 == accepted
        })
    })
}

/// Filters on the `--name` argument.
///
/// Panics when `fnmatch` provides an error code.
/// Unsafe: handles interacting with the system fnmatch library.
fn name_predicate(predicate: Predicate, name: CString) -> Predicate {
    Box::new(move |p, m| {
        Ok(predicate(p, m)? && {
            let path = p.components().last().unwrap().as_os_str();
            let path = unsafe { CString::from_vec_unchecked(path.as_bytes().to_vec()) };
            let result = unsafe {
                fnmatch(
                    name.as_ptr() as _,
                    path.as_bytes_with_nul().as_ptr() as _,
                    0,
                )
            };
            if result == 0 {
                true
            } else if result == unsafe { FNM_NOMATCH } {
                false
            } else {
                panic!("fnmatch failed")
            }
        })
    })
}

/// Filters on the `--exec` predicate.
///
/// print_anyway corrosponds to the -print command. It corrosponds to running
/// the command, but ignoring the result.
fn exec_predicate(predicate: Predicate, command: String, print_anyway: bool) -> Predicate {
    Box::new(move |p, m| {
        Ok(predicate(p, m)? && {
            match Exec::shell(command.to_string().replace("{}", &p.to_string_lossy())).join() {
                Ok(t) => t.success() && print_anyway,
                Err(e) => {
                    Error::Custom(&e).sig();
                    false
                }
            }
        })
    })
}

/// We should signal an error when crawling the path.
type SigError = bool;

/// Recursivly traverse `path`. Only adds if `predicate(path, metadata(path))`
/// returns true. If a file is a directory, `crawl_path` will still traverse.
/// Symlinks will be followed if `follow_syms` is `true`.
pub fn crawl_path(
    path: &Path,
    predicate: &Predicate,
    follow_syms: bool,
    visited: &mut HashSet<u64>,
) -> Result<SigError, io::Error> {
    let meta = if follow_syms {
        std::fs::metadata(path)?
    } else {
        std::fs::symlink_metadata(path)?
    };
    if predicate(path, &meta)? {
        println!("{}", path.display());
    }
    let mut sig_error = false;
    if meta.is_dir()
        && (follow_syms || !meta.file_type().is_symlink())
        && visited.insert(meta.ino())
    // This tests if meta.ino() is already in
    // visited
    {
        for fs in std::fs::read_dir(path)? {
            let fs = fs?.path();
            match crawl_path(&fs, predicate, follow_syms, visited) {
                Err(e) => {
                    match e.kind() {
                        io::ErrorKind::NotFound => println!("{}", fs.display()),
                        _ => Error::from_io(e, fs.display()).sig(),
                    }
                    sig_error = true;
                }
                Ok(sig) => sig_error = sig,
            }
        }
    }
    Ok(sig_error)
}

/// Takes args given and forms a predicate to correctly filter them.
pub fn form_predicate(opts: &ArgMatches) -> Predicate {
    // Default predicate: everything passes.
    let mut predicate: Predicate = Box::new(|_, _| Ok(true));
    if let Some(types) = opts.values_of("type").take() {
        // Apply type arg
        predicate = type_predicate(predicate, types.map(|f| f.to_string()).collect());
    }
    if let Some(names) = opts.values_of("name").take() {
        // apply name are
        predicate = names.fold(predicate, |predicate, name| {
            let name = unsafe { CString::from_vec_unchecked(name.as_bytes().to_vec()) };
            name_predicate(predicate, name)
        })
    }
    if let Some(mtimes) = opts.values_of("mtime").take() {
        predicate = mtimes.fold(predicate, |predicate, mtime| {
            time_predicate(predicate, mtime.parse().unwrap())
        })
    }
    if let Some(execs) = opts.values_of("exec").take() {
        let print_anyway = opts.is_present("print");
        predicate = execs.fold(predicate, |predicate, exec| {
            exec_predicate(predicate, exec.to_string(), print_anyway)
        })
    }
    predicate
}

/// Provides gnu-find compatible error handling.
#[must_use]
pub enum Error<T: std::fmt::Display> {
    /// Signals the too many symlinks error, expects to be given the path it tried to seach.
    TooManySymlinks(T),
    /// Signals whatever error it was given.
    Custom(T),
    /// Signals no such file was found, expects to be given the file searched for.
    NoSuchFile(T),
}

impl<T: std::fmt::Display> std::fmt::Display for Error<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Error::TooManySymlinks(s) => {
                write!(f, "‘{}’: Too many levels of symbolic links", s)
            }
            Error::Custom(s) => {
                write!(f, "{}", s)
            }
            Error::NoSuchFile(s) => {
                write!(f, "‘{}’: No such file or directory", s)
            }
        }
    }
}

impl<T: std::fmt::Display> Error<T> {
    /// Signal an error to the user. Does not exit.
    pub fn sig(&self) {
        let name = if cfg!(target_os = "linux") {
            std::env::args_os()
                .next()
                .unwrap_or(std::ffi::OsString::from("myfind"))
        } else {
            std::ffi::OsString::from("gfind")
        };
        eprintln!("{}: {}", name.to_string_lossy(), self);
    }

    /// Generates an error from an io::Error. If no error is found, a custom
    /// error is generated with whatever message `error` gives.
    pub fn from_io(error: io::Error, path: T) -> Error<String> {
        #[cfg(target_os = "linux")]
        let too_many_syms = Some(40);
        #[cfg(not(target_os = "linux"))]
        let too_many_syms = Some(62);
        match error.kind() {
            // The only way to examine a file that doesn't exist is to fail at
            // the first search queary, or to have the file system change while
            // a search is occuring. While the second is possible, I don't think
            // it likely to be tested.
            std::io::ErrorKind::NotFound => Error::NoSuchFile(format!("{}", path)),
            std::io::ErrorKind::Other if error.raw_os_error() == too_many_syms => {
                Error::TooManySymlinks(format!("{}", path))
            }
            _ => Error::Custom(format!("{}", error)),
        }
    }
}

/// Clap is opinionated about how it accepts arguments. We thus preprocess our
/// arguments. This handles the weird mechanic of exec, as well as changing
/// -flag to --flag. Happily, normal flag use still works, and exec can still be
/// passed a single argument as long as it's called with `--exec`.
///
/// If parsing was successful, then we return ok. Otherwise we return the string
/// to be printed out when parsing fails.
pub fn preprocess_args<I, S>(args: I) -> Result<Vec<String>, &'static str>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut out: Vec<String> = Vec::new();
    let mut exec: Option<Vec<String>> = None;
    for arg in args.into_iter() {
        let arg = arg.into();
        if let Some(mut cmd) = exec {
            if &arg == ";" {
                out.push(cmd.join(" "));
                exec = None;
            } else {
                exec = Some({
                    cmd.push(arg);
                    cmd
                });
            }
        } else {
            let sarg: &str = &arg;
            match sarg {
                "-print" => out.push(String::from("--print")),
                "-name" => out.push(String::from("--name")),
                "-type" => out.push(String::from("--type")),
                "-mtime" => out.push(String::from("--mtime")),
                "-exec" => {
                    out.push(String::from("--exec"));
                    exec = Some(Vec::new());
                }
                _ => out.push(arg.to_string()),
            }
        }
    }
    if exec.is_some() {
        return Err("missing argument to `-exec'");
    }

    Ok(out)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn preprocess() {
        let start = [
            "./filename",
            "-name",
            "thing*",
            "-exec",
            "cmd",
            "-type",
            ";",
            "-type",
            "b",
        ];
        assert_eq!(
            preprocess_args(start.iter().map(|s| s.to_string())).unwrap(),
            vec![
                "./filename",
                "--name",
                "thing*",
                "--exec",
                "cmd -type",
                "--type",
                "b"
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
        );
    }
}
