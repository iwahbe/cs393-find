use clap::{App, Arg, ArgMatches};
use std::fs::Metadata;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::path::Path;
use std::path::PathBuf;

use std::ffi::CString;

use fnmatch_sys::{fnmatch, FNM_NOMATCH};
use subprocess::Exec;

fn getopts() -> ArgMatches {
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
                .value_name("command")
                .conflicts_with("print"),
        )
        .arg(
            Arg::new("print").long("print").about(
                "True; print the full file name on the standard output, followed by a newline",
            ),
        )
        .get_matches()
    // TODO: provide arg preprocessor to handle arguments starting with - instead of --
    // Eample -name instead of --name
}

type Predicate = Box<dyn Fn(&Path, &Metadata) -> bool>;

fn type_predicate(predicate: Predicate, accepted: Vec<String>) -> Predicate {
    Box::new(move |p, m: &Metadata| {
        let ft = m.file_type();
        predicate(p, m)
            && if ft.is_block_device() {
                accepted.contains(&String::from("b"))
            } else if ft.is_char_device() {
                accepted.contains(&String::from("c"))
            } else if ft.is_dir() {
                accepted.contains(&String::from("d"))
            } else if ft.is_fifo() {
                accepted.contains(&String::from("p"))
            } else if ft.is_file() {
                accepted.contains(&String::from("f"))
            } else if ft.is_symlink() {
                accepted.contains(&String::from("l"))
            } else if ft.is_socket() {
                accepted.contains(&String::from("s"))
            } else {
                panic!("Found unimplemented type")
            }
    })
}

fn time_predicate(predicate: Predicate, accepted: i32) -> Predicate {
    Box::new(move |p, m: &Metadata| {
        predicate(p, m) && {
            let modified = m.modified().unwrap();
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
        }
    })
}

fn name_predicate(predicate: Predicate, name: CString) -> Predicate {
    Box::new(move |p, m| {
        predicate(p, m) && {
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
        }
    })
}

fn exec_predicate(predicate: Predicate, command: String) -> Predicate {
    Box::new(move |p, m| {
        predicate(p, m) && {
            Exec::shell(command.to_string().replace("{}", &p.to_string_lossy()))
                .join()
                .unwrap()
                .success()
        }
    })
}

fn main() -> std::io::Result<()> {
    let opts = getopts();
    let starting_point = {
        let p = Path::new(opts.value_of("starting_point").unwrap_or("."));
        if opts.is_present("C") {
            p.canonicalize().unwrap()
        } else {
            p.to_path_buf()
        }
    };
    // Default predicate: everything passes.
    let mut predicate: Predicate = Box::new(|_, _| true);
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
    for p in crawl_path(&starting_point, &predicate, opts.is_present("L"))? {
        println!("{}", p.display());
    }
    Ok(())
}

fn crawl_path(
    path: &Path,
    predicate: &Predicate,
    follow_syms: bool,
) -> Result<impl Iterator<Item = PathBuf>, std::io::Error> {
    let meta = if follow_syms {
        std::fs::metadata(path)?
    } else {
        std::fs::symlink_metadata(path)?
    };
    let mut out = Vec::new();
    if predicate(path, &meta) {
        out.push(path.to_path_buf());
    }
    if path.is_dir() {
        for fs in std::fs::read_dir(path).unwrap() {
            out.extend(crawl_path(&fs.unwrap().path(), predicate, follow_syms)?)
        }
    }
    Ok(out.into_iter())
}
