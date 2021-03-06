// First, we declare all external dependencies we need here
#[macro_use]
extern crate clap;
#[macro_use] // Has in fact only macros which generate types for us
extern crate error_chain;
extern crate hyper;
extern crate thread_scoped;
// Next, import actual symbols and modules we need
use std::cmp;
use std::fs;
// NB: to use Read and Write traits, we need to bring them into scope explicitly
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::exit;
// Same as for Read/Write, or we won't be able to do usize::from_str later
use std::str::FromStr;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;

use hyper::status::StatusCode;

use thread_scoped::scoped;

// A small helper macro which is like println! but for stderr
macro_rules! errorln {
    ($($arg:tt)*) => { let _ = writeln!(io::stderr(), $($arg)*); };
}
// Program starting point, as usual
fn main() {
    // First, parse arguments
    let Args { dest_dir, list_file, threads_num, speed_limit } = parse_args();
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        // Open file with list of files to download
        let mut fd = match fs::File::open(&list_file) {
            Ok(val) => val,
            Err(err)  => {
                errorln!("Failed to open {}: {}", list_file, err);
                exit(1)
            }
        };
        // Then read all of its contents into buffer
        let mut text = String::new();
        if let Err(err) = fd.read_to_string(&mut text) {
            errorln!("Failed to read contents of {}: {}", list_file, err);
            exit(1)
        }
        text
    };
    // Next, we split the whole file into lines in-place
    // And for each line which contains proper url-filename tuple,
    // We yield that tuple
    let files_seq = all_text
        .lines()
        .filter_map(|line| {
            let mut pieces = line.split(|c| " \r\n\t".contains(c)).filter(|s| !s.is_empty());
            let url = pieces.next();
            let filename = pieces.next();
            if let (Some(url_value), Some(fname_value)) = (url, filename) {
                Some((url_value, fname_value))
            }
            else { None }
        })
        .fuse();
    // To consume this iterator in case of multithreading, we put it under mutex
    let files_seq = Mutex::new(files_seq);
    // Also, construct token bucket to control download speed
    // And put it under mutex likewise
    let bucket = Mutex::new(TokenBucket::new(speed_limit));
    // Pool for N-1 worker thread guards 
    let mut worker_guards = Vec::with_capacity(threads_num - 1);
    // Finally, create worker threads
    for i in 1..threads_num {
        // A minor annoyance - we need to create separate reference variables
        let seq_ref = &files_seq;
        let bucket_ref = &bucket;
        let dest_dir_ref = &dest_dir;
        // Create scoped worker thread and put its guard object to vector
        worker_guards.push(
            unsafe { scoped(move || pull_files(i, dest_dir_ref, bucket_ref, seq_ref)) }
        );
    }
    // Main thread would do just the same as worker ones, summing up to N threads
    pull_files(0, &dest_dir, &bucket, &files_seq);
    // Vector of guards will be dropped right here
}
// Pre-parsed arguments received from command line
struct Args
{
    dest_dir:    String,
    list_file:   String,
    threads_num: usize,
    speed_limit: usize,
}

mod parse_args_errors {
    error_chain! {
        types {}
        links {}
        foreign_links {
            Io(::std::io::Error);
            ParseInt(::std::num::ParseIntError);
        }
        errors {
            ArgError(arg: String) {
                description("Error parsing command line argument")
                display("Error parsing command line argument <{}>", arg)
            }
        }
    }
}
// Parse command line arguments
fn parse_args() -> Args {
    // Import errors definitions related to parsing arguments
    use clap::Arg;
    use parse_args_errors::*;
    // Define command line parser
    // NB: app_from_crate macro simply sets several useful defaults
    let args = app_from_crate!()
        .arg(Arg::with_name("dest_dir")
            .help("Destination dir where to store downloaded files")
            .short("o")
            .required(true)
            .takes_value(true)
        )
        .arg(Arg::with_name("list_file")
            .help("File which contains list of URLs to download and local names for files")
            .short("f")
            .required(true)
            .takes_value(true)
        )
        .arg(Arg::with_name("threads_num")
            .help("Number of worker threads to use")
            .short("n")
            .default_value("1")
        )
        .arg(Arg::with_name("speed_limit")
            .help("Global speed limit, in bytes per second. 0 means no limit.
Suffixes supported:
    k, K - kilobytes, i.e. 1024's of bytes
    m, M - megabytes, i.e. 1024*1024's of bytes
")
            .short("l")
            .default_value("0")
        )
        .get_matches();
    // Next, perform additional parsing of arguments
    // And report any errors which can happen
    // We use several functions to have proper errors nesting
    return match process_args(&args) {
        Ok(value) => value,
        Err(err)  => {
            errorln!("Error: {}", err);
            for e in err.iter().skip(1) {
                errorln!("  Caused by: {}", e);
            }
            errorln!("{}", args.usage());
            exit(1);
        }
    };
    // Simply construct Args and return any error if one occurs
    fn process_args(args: &clap::ArgMatches) -> Result<Args> {
        Ok(Args {
            dest_dir:    arg_parse(args, "dest_dir",    arg_dest_dir)?,
            list_file:   arg_parse(args, "list_file",   arg_list_file)?,
            threads_num: arg_parse(args, "threads_num", arg_threads_num)?,
            speed_limit: arg_parse(args, "speed_limit", arg_speed_limit)?,
        })
    }
    // Small helper func which forwards any errors during parsing of specific
    // argument and appends proper context info
    // Argument value is unwrapped from Option, since our arguments are either required
    // or have default value
    fn arg_parse<T, F>(args: &clap::ArgMatches, name: &str, func: F) -> Result<T>
        where F: FnOnce(&str) -> Result<T>
    {
        // Simply invoke argument parser and wrap any Err passing by with chain_err
        func(args.value_of(name).unwrap()).chain_err(|| ErrorKind::ArgError(name.to_owned()))
    }
    // Check that destination path is a directory and exists
    fn arg_dest_dir(arg: &str) -> Result<String> {
        if fs::metadata(arg)?.is_dir() {
            Ok(arg.to_owned())
        }
        else {
            bail!("{}: not a directory", arg)
        }
    }
    // Check that surces list exists and is a file
    fn arg_list_file(arg: &str) -> Result<String> {
        if fs::metadata(arg)?.is_file() {
            Ok(arg.to_owned())
        }
        else {
            bail!("{}: not a file", arg)
        }
    }
    // Get number of threads as number greater than 0, default 1
    fn arg_threads_num(arg: &str) -> Result<usize> {
        match usize::from_str(arg)? {
            0 => bail!("cannot be zero"),
            n => Ok(n)
        }
    }
    // Get speed limit as a number with some custom prefixes
    fn arg_speed_limit(arg: &str) -> Result<usize> {
        match arg.char_indices().last() {
            None => Ok(0),
            Some((last_index, last_char)) => {
                // Set multiplier based on speed limit suffix
                let mult: usize = match last_char {
                    'k' | 'K' => 1024,
                    'm' | 'M' => 1024 * 1024,
                    _ => 1
                };
                // Next, get actual number string based on multiplier being recognized or not
                let num_str = if mult == 1 { arg } else { arg.split_at(last_index).0 };
                // We could map error, but it's also possible to use '?'
                // and simply return result wrapped into Ok 
                Ok(usize::from_str(num_str).map(|n| n * mult)?)
            }
        }
    }
}

struct TokenBucket {
    fill_rate: usize,
    capacity:  usize,
    remaining: f64,
    timestamp: Instant,
}

impl TokenBucket {
    fn new(rate: usize) -> TokenBucket {
        TokenBucket::with_capacity(rate, rate)
    }

    fn with_capacity(rate: usize, capacity: usize) -> TokenBucket {
        TokenBucket {
            fill_rate: rate,
            capacity:  capacity,
            remaining: 0f64,
            timestamp: Instant::now(),
        }
    }

    fn take(&mut self, amount: usize) -> usize {
        // 0. For zero fillrate, treat this bucket as infinite
        if self.fill_rate == 0 {
            return amount;
        }
        // 1. Add to bucket rate / delta
        let delta = {
            let now = Instant::now();
            now - std::mem::replace(&mut self.timestamp, now)
        };
        let delta_fill = ((delta.as_secs() as f64) + (delta.subsec_nanos() as f64) / 1_000_000_000f64) * (self.fill_rate as f64);
        self.remaining = (self.remaining + delta_fill).min(self.capacity as f64);
        // 2. Take as much as possible from bucket, but no more than is present there
        let taken = cmp::min(self.remaining.floor() as usize, amount);
        self.remaining = (self.remaining - (taken as f64)).max(0f64);
        return taken;
    }
}

fn pull_files<'a, I>(thread_num: usize, dest_dir: &str, bucket: &Mutex<TokenBucket>, list: &Mutex<I>)
    where I: Iterator<Item = (&'a str, &'a str)> + Send
{
    loop {
        // Having this as separate expression should prevent locking for the whole duration
        let (url, dest_path) = match list.lock().unwrap().next() {
            None => break,
            Some((url, dest)) => (url, Path::new(dest_dir).join(dest)) 
        };
        println!("#{}: Downloading {} -> {}", thread_num, url, dest_path.display());
        if let Err(error) = pull_file(url, &dest_path, bucket) {
            errorln!("#{}: Failed {} -> {} due to:\n    {}", thread_num, url, dest_path.display(), error);
        }
    }
}

quick_error! {
    #[derive(Debug)]
    enum DownloadError {
        Hyper(error: hyper::Error) {
            description("HTTP request error")
            cause(error)
            display(me) -> ("{}: {}", me.description(), error)
            from()
        }
        Server(status: StatusCode) {
            description("HTTP request error")
            display(me) -> ("{}: code {}", me.description(), status)
        }
        Io(error: std::io::Error) {
            description("I/O error")
            cause(error)
            display(me) -> ("{}: {}", me.description(), error)
            from()
        }
    }
}

fn pull_file(src_url: &str, dest_path: &Path, bucket: &Mutex<TokenBucket>) -> Result<(), DownloadError> {
    let mut response = hyper::Client::new().get(src_url).send()?;
    if response.status != hyper::status::StatusCode::Ok {
        return Err(DownloadError::Server(response.status));
    }
    let mut dest_file = fs::File::create(&dest_path)?;
    let _ = copy_limited(&mut response, &mut dest_file, bucket)?;
    Ok(())
}

fn copy_limited<R: Read + ?Sized, W: Write + ?Sized>(reader: &mut R, writer: &mut W, bucket: &Mutex<TokenBucket>) -> io::Result<u64> {
    let mut buf = [0; 64 * 1024];
    let mut written = 0;
    loop {
        let limit = bucket.lock().unwrap().take(buf.len());
        if limit == 0 {
            thread::yield_now();
            continue;
        }
        let mut part = &mut buf[..limit];
        let len = match reader.read(&mut part) {
            Ok(0) => return Ok(written),
            Ok(len) => len,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        writer.write_all(&mut part[..len])?;
        written += len as u64;
    }
}
