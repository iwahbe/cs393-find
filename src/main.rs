use clap::{App, Arg, ArgMatches};
use std::collections::HashSet;
use std::fs::Metadata;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt};
use std::path::Path;
use std::path::PathBuf;

use std::ffi::CString;

use fnmatch_sys::{fnmatch, FNM_NOMATCH};
use subprocess::Exec;

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
            preprocess_args(start.iter().map(|s| s.to_string())),
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

/// Process command line arguments into a usefull output.
/// This uses the `Clap` library.
fn getopts(preprocessed_args: Vec<String>) -> ArgMatches {
    App::new("Something like unix `find` command.")
        .version("0.1")
        .author("Ian Wahbe <github @iwahbe>")
        .about("An reimplimentation of find for scholastic purposes.")
        .arg(Arg::new("L").short('L').about("Follow symbolic links"))
        .arg(Arg::new("C").short('C').about("Canonicalize paths"))
        .arg(Arg::new("starting_point").about("Starting point for search"))
        .arg(
            Arg::new("name")
                .long("name")
                .value_name("pattern")
                .about("Filters file names according to a glob expanded path.")
                .takes_value(true),
        )
        .arg(
            Arg::new("mtime")
                .long("mtime")
                .about(
                    "File's data was last modified less than, more than or exactly n*24 hours ago",
                )
                .takes_value(true)
                .value_name("n")
                .validator(|s| match s.parse::<i32>() {
                    Ok(_) => Ok(()),
                    Err(e) => Err(e),
                })
                .allow_hyphen_values(true),
        )
        .arg(
            Arg::new("type")
                .long("type")
                .takes_value(true)
                .value_name("t")
                .possible_values(&["b", "c", "d", "p", "f", "l", "s"])
                .require_delimiter(true),
        )
        .arg(
            Arg::new("exec")
                .long("exec")
                .about(
                    "Execute `command`; true if 0 status is returned. All \
                                 following arguments to find are taken to be arguments \
                                 to the command until an argument consisting of `;' is \
                                 encountered. The string `{}' is replaced by the current \
                                 file name being processed everywhere it occurs in the \
                                 arguments to the command.",
                )
                .takes_value(true)
                .value_name("command"),
        )
        .arg(
            Arg::new("print").long("print").about(
                "True; print the full file name on the standard output, followed by a newline",
            ),
        )
        .get_matches_from(preprocessed_args)
}

/// Clap is opinionated about how it accepts arguments. We thus preprocess our
/// arguments. This handles the weird mechanic of exec, as well as changing
/// -flag to --flag. Happily, normal flag use still works, and exec can still be
/// passed a single argument as long as it's called with `--exec`.
fn preprocess_args<I, S>(args: I) -> Vec<String>
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

    out
}

/// Describes a command line given predicate.
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
            let modified = m.modified()?;
            let now = std::time::SystemTime::now();
            let time_delta = if modified > now {
                modified.duration_since(now).unwrap().as_secs()
            } else {
                now.duration_since(modified).unwrap().as_secs()
            } as f64
                / 60.0 // to minutes
                / 60.0 // To hours
                / 24.0; // To days
            if accepted == 0 {
                time_delta < 1.0
            } else if accepted > 0 {
                time_delta > accepted as f64
            } else {
                time_delta < (-accepted) as f64
            }
        })
    })
}

/// Filters on the `--name` argument.
///
/// Panics when `fnmatch` provides an error code.
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
/// Panics when the pipe fails.
fn exec_predicate(predicate: Predicate, command: String) -> Predicate {
    Box::new(move |p, m| {
        Ok(predicate(p, m)? && {
            Exec::shell(command.to_string().replace("{}", &p.to_string_lossy()))
                .join()
                .unwrap()
                .success()
        })
    })
}

/// Recursivly traverse `path`. Only adds if `predicate(path, metadata(path))`
/// returns true. If a file is a directory, `crawl_path` will still traverse.
/// Symlinks will be followed if `follow_syms` is `true`.
fn crawl_path(
    path: &Path,
    predicate: &Predicate,
    follow_syms: bool,
    visited: &mut HashSet<u64>,
) -> Result<impl Iterator<Item = PathBuf>, io::Error> {
    let meta = if follow_syms {
        std::fs::metadata(path)?
    } else {
        std::fs::symlink_metadata(path)?
    };
    let mut out = Vec::new();
    if predicate(path, &meta)? {
        out.push(path.to_path_buf());
    }
    if meta.is_dir()
        && (follow_syms || !meta.file_type().is_symlink())
        && visited.insert(meta.ino())
    {
        for fs in std::fs::read_dir(path)? {
            let fs = fs?.path();
            if !fs.exists() && predicate(&fs, &std::fs::symlink_metadata(&fs)?)? {
                out.push(fs.to_path_buf());
            } else {
                out.extend(crawl_path(&fs, predicate, follow_syms, visited)?)
            }
        }
    }
    Ok(out.into_iter())
}

fn main() -> io::Result<()> {
    let args = std::env::args();
    let pre_process = preprocess_args(args);
    let opts = getopts(pre_process);
    let starting_point = {
        let p = Path::new(opts.value_of("starting_point").unwrap_or("."));
        if opts.is_present("C") {
            p.canonicalize().unwrap()
        } else {
            p.to_path_buf()
        }
    };
    // Default predicate: everything passes.
    let mut predicate: Predicate = Box::new(|_, _| Ok(true));
    if let Some(types) = opts.values_of("type").take() {
        // Apply type arg
        predicate = type_predicate(predicate, types.map(|f| f.to_string()).collect());
    }
    if let Some(name) = opts.value_of("name").take() {
        // apply name are
        let name = unsafe { CString::from_vec_unchecked(name.as_bytes().to_vec()) };
        predicate = name_predicate(predicate, name);
    }
    if let Some(mtime) = opts.value_of("mtime").take() {
        predicate = time_predicate(predicate, mtime.parse().unwrap());
    }
    if let Some(exec) = opts.value_of("exec").take() {
        predicate = exec_predicate(predicate, exec.to_string());
    }
    if opts.is_present("print") {
        // we print everything anyway
        predicate = Box::new(move |p, m| {
            predicate(p, m)?;
            Ok(true)
        });
    }
    let mut visited = HashSet::new();
    if !starting_point.exists() {
        eprintln!(
            "find: {}: No such file or directory",
            starting_point.display()
        );
        return Ok(());
    }
    match crawl_path(
        &starting_point,
        &predicate,
        opts.is_present("L"),
        &mut visited,
    ) {
        Ok(paths) => {
            for p in paths {
                println!("{}", p.display());
            }
        }
        Err(error) => match error.kind() {
            // The only way to examine a file that doesn't exist is to fail at
            // the first search queary, or to have the file system change while
            // a search is occuring. While the second is possible, I don't think
            // it likely to be tested.
            std::io::ErrorKind::NotFound => eprintln!(
                "find: {}: No such file or directory",
                starting_point.display()
            ),
            std::io::ErrorKind::Other if error.raw_os_error() == Some(62) => eprintln!(
                "find: {}: Too many levels of symbolic links",
                starting_point.display()
            ),
            _ => Err(error)?,
        },
    }
    Ok(())
}
