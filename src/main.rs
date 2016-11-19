#[macro_use]
extern crate clap;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate hyper;
extern crate thread_scoped;

use std::process::exit;
use std::fmt::Arguments;
use std::fs;
use std::io::{self, Read, Write};
use std::str::FromStr;
use std::sync::{Mutex};

use thread_scoped::scoped;

macro_rules! unoption_or {
    ($source:expr => $on_none:expr) => (
        match $source {
            Some(val) => val,
            None => $on_none
        }
    )
}

macro_rules! unresult_or {
    ($source:expr => || $on_err:expr) => (
        match $source {
            Ok(val)  => val,
            Err(_) => $on_err
        }
    );
    ($source:expr => |$var:ident| $on_err:expr) => (
        match $source {
            Ok(val)  => val,
            Err($var) => $on_err
        }
    )
}

fn pull_files<'a, I>(thread_num: usize, dest_dir: &'a str, list: &'a Mutex<I>)
    where I: Iterator<Item = (&'a str, &'a str)> + Send
{
    info!("worker thread #{} started", thread_num);
    loop {
        // Having this as separate expression should prevent locking for the whole duration
        let (url, dest) = unoption_or!(list.lock().unwrap().next() => break);
        debug!("#{}: {} -> {}/{}\n", thread_num, url, dest_dir, dest);
        let response = unresult_or!(hyper::Client::new().get(url).send() =>
            |err| {
                error!("#{}: request {} -> {}/{} failed: {}", thread_num, url, dest_dir, dest, err);
                continue;
            }
        );
    }
    debug!("worker thread #{} finished", thread_num);
}

fn main() {
    let _ = env_logger::init().unwrap();
    // First, configure our command line
    let args = clap_app!(httpdl =>
        (version: crate_version!())
        (author:  crate_authors!())
        (about: "Downloads files via HTTP")
        (@arg directory:   -o +required +takes_value
            "Directory where to place downloaded files"
        )
        (@arg list_file:   -f +required +takes_value
            "File which contains list of URLs to download and the local file names to use"
        )
        (@arg speed_limit: -l +takes_value
            "Limit download speed to specified value, in bytes; suffixes like 'k' (kilobytes) and 'm' (megabytes)"
        )
        (@arg threads_num: -n +takes_value
            "Download files using N threads"
        )
    ).get_matches();
    // Complains about error processing specified arg, then dumps usage and exits with error
    let fail_arg = |name: &str, format: Arguments| -> ! {
        let _ = writeln!(&mut io::stderr(), "<{}>: {}\n{}", name, format, &args.usage());
        exit(1)
    };

    // Read and validate destination dir
    // NB: can unwrap because it's required and thus can't be None
    let dest_dir    = args.value_of("directory").unwrap();
    {
        let meta = unresult_or!(fs::metadata(dest_dir) =>
            || fail_arg("directory", format_args!("'{}' does not exist or inaccessible", dest_dir))
        );
        if !meta.is_dir() {
            fail_arg("directory", format_args!("'{}' is not a directory", dest_dir))
        }
    }
    // Read and validate path to lists file
    // NB: can unwrap because it's required and thus can't be None
    let list_file   = args.value_of("list_file").unwrap();
    {
        let meta = unresult_or!(fs::metadata(list_file) =>
            || fail_arg("list_file", format_args!("'{}' does not exist or inaccessible", list_file))
        );
        if !meta.is_file() {
                fail_arg("list_file", format_args!("'{}' is not a file", list_file))
        }
    }
    // Read and parse download speed limit
    let speed_limit = match args.value_of("speed_limit") {
        None => 0,
        Some(_) => fail_arg("speed_limit", format_args!("not supported for now, sorry"))
    };
    // Read and parse number of threads which should be used
    let threads_num = match args.value_of("threads_num") {
        None => 1usize,
        Some(value) => {
            let num = unresult_or!(usize::from_str(value) =>
                || fail_arg("threads_num", format_args!("'{}' cannot be parsed as unsigned integer", value))
            );
            if num == 0 {
                fail_arg("threads_num", format_args!("cannot be zero"));
            }
            num
        }
    };

    // Now, we read whole list file and then fill files mapping
    let all_text = {
        let mut fd = unresult_or!(fs::File::open(list_file) =>
            || fail_arg("list_file", format_args!("failed to open"))
        );
        let mut text = String::new();
        let _ = unresult_or!(fd.read_to_string(&mut text) =>
            || fail_arg("list_file", format_args!("failed to read"))
        );
        text
    };
    let mut files_map: Vec<(&str, &str)> = Vec::new();
    // Next iterate all lines in file and get URLs and file names from them
    for line in all_text.lines() {
        let mut pieces = line.split(|c| " \r\n\t".contains(c)).filter(|s| !s.is_empty());
        let url = pieces.next();
        let filename = pieces.next();
        if let (Some(url_value), Some(fname_value)) = (url, filename) {
            files_map.push((url_value, fname_value));
        }
    }
    // Finally, when we just need to consume all this in random order,
    // transform it into consuming iterator and pack under mutex
    let files_seq = Mutex::new(files_map.into_iter().fuse());
    // Now, create N - 1 worker threads and each will pull files
    // Looks simpler than fancy tricks like recursive guards
    let mut worker_guards = Vec::with_capacity(threads_num);
    for i in 1..threads_num {
        let seq_ref = &files_seq; // thus we can move reference to seq into closure
        worker_guards.push(
            unsafe { scoped(move || pull_files(i, dest_dir, seq_ref)) }
        );
    }
    // Main thread would do just the same as worker ones, summing up to N threads
    pull_files(0, dest_dir, &files_seq);
    // Vector of guards will stop right here
}
