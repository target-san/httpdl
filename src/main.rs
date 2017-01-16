// First, we declare all external dependencies we need here
#[macro_use] // Tells compiler to bring macros from that crate into our scope
extern crate rustc_decodable;
extern crate docopt;
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
// Let's write a comprehensive USAGE
const USAGE: &'static str = "
HTTP Downloader
Simplistic console file downloader with multithreading and speed limiting
Pulls data from HTTP only

Usage:
    httpdl -f=<list_file> -o=<dest_dir> [-n=<threads_num>] [-l=<speed_limit>]
    httpdl --help

Options:
    -f=<list_file>      File which contains list of URLs and respective local file names
    -o=<dest_dir>       Destination directory where all the files from <list_file> will be
                        downloaded to
    -n=<threads_num>    Number of threads to utilize
                        [default: 1]
    -l=<speed_limit>    Limit total download speed; suffixes supported:
                        k, K - number of kilobytes (1024) per second
                        m, M - number of megabytes (1024*1024) per second
                        [default: 0] - no speed limit
";

#[derive(RustcDecodable)]
struct DocoptArgs {
    arg_dest_dir:       String,
    arg_list_file:      String,
    arg_threads_num:    usize,
    arg_speed_limit:    String,
}

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
    use parse_args_errors::*;
    // First, configure our command line - use macros from clap crate
    let args = clap_app!(httpdl =>
        (version: crate_version!())
        (author:  crate_authors!())
        (about: "Downloads files via HTTP")
        (@arg dest_dir:   -o +required +takes_value
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
    fn process_args() -> Result<Args> {
        Ok(Args {
            dest_dir:    arg_parse(args, "dest_dir",    arg_dest_dir)?,
            list_file:   arg_parse(args, "list_file",   arg_list_file)?,
            threads_num: arg_parse(args, "threads_num", arg_threads_num)?,
            speed_limit: arg_parse(args, "speed_limit", arg_speed_limit)?,
        })
    }
    // Small helper func which forwards any errors during parsing of specific
    // argument and appends proper context info
    fn arg_parse<T, F>(args: &clap::ArgMatches, name: &str, func: F) -> Result<T>
        where F: FnOnce(Option<&str>) -> Result<T>
    {
        // Simply invoke argument parser and wrap any Err passing by with chain_err
        func(args.value_of(name)).chain_err(|| ErrorKind::ArgError(name.to_owned()))
    }
    // Check that destination path is a directory and exists
    fn arg_dest_dir(arg: Option<&str>) -> Result<String> {
        match arg {
            None => bail!("missing argument"),
            Some(s) =>
                if fs::metadata(s)?.is_dir() { Ok(s.to_owned()) }
                else { bail!("{}: not a directory", s) }
        }
    }
    // Check that surces list exists and is a file
    fn arg_list_file(arg: Option<&str>) -> Result<String> {
        match arg {
            None => bail!("missing argument"),
            Some(s) => 
                if fs::metadata(s)?.is_file() { Ok(s.to_owned()) }
                else { bail!("{}: not a file", s) }
        }
    }
    // Get number of threads as number greater than 0, default 1
    fn arg_threads_num(arg: Option<&str>) -> Result<usize> {
        match arg {
            None => Ok(1),
            Some(s) => match usize::from_str(s)? {
                0 => bail!("cannot be zero"),
                n => Ok(n)
            }
        }
    }
    // Get speed limit as a number with some custom prefixes
    fn arg_speed_limit(arg: Option<&str>) -> Result<usize> {
        match arg {
            None => Ok(0),
            Some(s) => match s.char_indices().last() {
                None => Ok(0),
                Some((last_index, last_char)) => {
                    // Set multiplier based on speed limit suffix
                    let mult: usize = match last_char {
                        'k' | 'K' => 1024,
                        'm' | 'M' => 1024 * 1024,
                        _ => 1
                    };
                    // Next, get actual number string based on multiplier being recognized or not
                    let num_str = if mult == 1 { s } else { s.split_at(last_index).0 };
                    // We could map error, but it's also possible to use '?'
                    // and simply return result wrapped into Ok 
                    Ok(usize::from_str(num_str).map(|n| n * mult)?)
                }
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
            errorln!("#{}: Failed {} -> {} due to: {}", thread_num, url, dest_path.display(), error);
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
