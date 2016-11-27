#[macro_use]
extern crate clap;
#[macro_use]
extern crate log;
#[macro_use]
extern crate quick_error;
extern crate env_logger;
extern crate hyper;
extern crate thread_scoped;

use std::cmp;
use std::error::Error;
use std::fmt::Arguments;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::exit;
use std::str::FromStr;
use std::sync::Mutex;
use std::thread;
use std::time::Instant;

use hyper::status::StatusCode;

use thread_scoped::scoped;

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

fn pull_files<'a, I>(thread_num: usize, dest_dir: &str, bucket: &Mutex<TokenBucket>, list: &Mutex<I>)
    where I: Iterator<Item = (&'a str, &'a str)> + Send
{
    debug!("worker thread #{} started", thread_num);
    loop {
        // Having this as separate expression should prevent locking for the whole duration
        let (url, dest_path) = match list.lock().unwrap().next() {
            None => break,
            Some((url, dest)) => (url, Path::new(dest_dir).join(dest)) 
        };
        info!("Thread #{}: Downloading {} -> {}", thread_num, url, dest_path.display());
        if let Err(error) = pull_file(url, &dest_path, bucket) {
            error!("#{} {} -> {} failed: {}", thread_num, url, dest_path.display(), error);
        }
    }
    debug!("worker thread #{} finished", thread_num);
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

fn main() {
    let _ = env_logger::init().unwrap();

    let Args { dest_dir, list_file, thread_num, speed_limit } = parse_args();
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        let mut fd = match fs::File::open(list_file) {
            Ok(val) => val,
            Err(_)  => fail_arg("list_file", format_args!("failed to open")) 
        };
        let mut text = String::new();
        if let Err(_) = fd.read_to_string(&mut text) {
            fail_arg("list_file", format_args!("failed to read"))
        }
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
    // Construct locked token bucket with specified limit
    let bucket = Mutex::new(TokenBucket::new(speed_limit));
    // Now, create N - 1 worker threads and each will pull files
    // Looks simpler than fancy tricks like recursive guards
    let mut worker_guards = Vec::with_capacity(threads_num);
    // Finally, create worker threads
    for i in 1..threads_num {
        let seq_ref = &files_seq; // thus we can move reference to seq into closure
        let bucket_ref = &bucket;
        worker_guards.push(
            unsafe { scoped(move || pull_files(i, dest_dir, bucket_ref, seq_ref)) }
        );
    }
    // Main thread would do just the same as worker ones, summing up to N threads
    pull_files(0, dest_dir, &bucket, &files_seq);
    // Vector of guards will stop right here
}

struct Args
{
    dest_dir:    String,
    list_file:   String,
    thread_num:  usize,
    speed_limit: usize,
}

quick_error! {
    #[derive(Debug)]
    enum ArgError {
        Arg(name: &str, error: Box<Error>) {
            cause(error)
            display("<{}>: {}", name, error)
        }
    }
}

fn parse_args() -> Args {
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

    let process_args = |args: &clap::ArgMatches| -> Result<Args, Box<Error>>{
        // Destination directory
        let dest_dir = args.value_of("directory").unwrap();
        if ! fs::metadata(dest_dir)?.is_dir() {
            return Err(format_args!("'{}' is not a directory", dest_dir).into());
        }
        // Source list file
        let list_file = args.value_of("list_file").unwrap();
        if ! fs::metadata(list_file)?.is_file() {
            return Err(format_args!("'{}' is not a file", dest_dir).into());
        }
        // Number of threads
        let threads_num = match args.value_of("threads_num") {
            None => 1usize,
            Some(value) => match usize::from_str(value) {
                Err(_)      => fail_arg("threads_num", format_args!("'{}' cannot be parsed as unsigned integer", value)),
                Ok(0usize)  => fail_arg("threads_num", format_args!("cannot be zero")),
                Ok(num)     => num 
            }
        };

        // Read and parse download speed limit
        let speed_limit = match args.value_of("speed_limit") {
            None => 0,
            Some(_) => fail_arg("speed_limit", format_args!("not supported for now, sorry"))
        };

        Ok(Args {
            dest_dir:    dest_dir,
            list_file:   list_file,
            threads_num: threads_num,
            speed_limit: speed_limit,
        })
    };

    match process_args(&args) {
        Ok(value) => value,
        Err(err)  => {

        }
    }
}