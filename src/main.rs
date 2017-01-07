#[macro_use]
extern crate clap;
#[macro_use]
extern crate log;
#[macro_use]
extern crate error_chain;
extern crate env_logger;
extern crate hyper;
extern crate thread_scoped;

use std::cmp;
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

fn main() {
    let _ = env_logger::init().unwrap();

    let Args { dest_dir, list_file, threads_num, speed_limit } = parse_args();
    // Now, we read whole list file and then fill files mapping
    let all_text = {
        let mut fd = match fs::File::open(&list_file) {
            Ok(val) => val,
            Err(err)  => {
                let _ = writeln!(io::stderr(), "failed to open {}: {}", list_file, err);
                exit(1)
            }
        };
        let mut text = String::new();
        if let Err(err) = fd.read_to_string(&mut text) {
            let _ = writeln!(io::stderr(), "failed to read contents of {}: {}", list_file, err);
            exit(1)
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
    let mut worker_guards = Vec::with_capacity(threads_num - 1);
    // Finally, create worker threads
    for i in 1..threads_num {
        let seq_ref = &files_seq; // thus we can move reference to seq into closure
        let bucket_ref = &bucket;
        let dest_dir_ref = &dest_dir;
        worker_guards.push(
            unsafe { scoped(move || pull_files(i, dest_dir_ref, bucket_ref, seq_ref)) }
        );
    }
    // Main thread would do just the same as worker ones, summing up to N threads
    pull_files(0, &dest_dir, &bucket, &files_seq);
    // Vector of guards will stop right here
}

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

fn parse_args() -> Args {
    use parse_args_errors::*;
    // First, configure our command line
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

    return match process_args(&args) {
        Ok(value) => value,
        Err(err)  => {
            let _ = writeln!(io::stderr(), "Error: {}", err);
            for e in err.iter().skip(1) {
                let _ = writeln!(io::stderr(), "  Caused by: {}", e);
            }
            let _ = writeln!(io::stderr(), "{}", args.usage());
            exit(1);
        }
    };

    fn process_args(args: &clap::ArgMatches) -> Result<Args> {
        Ok(Args {
            dest_dir:    arg_parse(args, "dest_dir",    arg_dest_dir)?,
            list_file:   arg_parse(args, "list_file",   arg_list_file)?,
            threads_num: arg_parse(args, "threads_num", arg_threads_num)?,
            speed_limit: arg_parse(args, "speed_limit", arg_speed_limit)?,
        })
    }

    fn arg_parse<T, F>(args: &clap::ArgMatches, name: &str, func: F) -> Result<T>
        where F: FnOnce(Option<&str>) -> Result<T>
    {
        func(args.value_of(name)).chain_err(|| ErrorKind::ArgError(name.to_owned()))
    }

    fn arg_dest_dir(arg: Option<&str>) -> Result<String> {
        match arg {
            None => bail!("missing argument"),
            Some(s) =>
                if fs::metadata(s)?.is_dir() { Ok(s.to_owned()) }
                else { bail!("{}: not a directory", s) }
        }
    }

    fn arg_list_file(arg: Option<&str>) -> Result<String> {
        match arg {
            None => bail!("missing argument"),
            Some(s) => 
                if fs::metadata(s)?.is_file() { Ok(s.to_owned()) }
                else { bail!("{}: not a file", s) }
        }
    }

    fn arg_threads_num(arg: Option<&str>) -> Result<usize> {
        match arg {
            None => Ok(1),
            Some(s) => match usize::from_str(s)? {
                0 => bail!("cannot be zero"),
                n => Ok(n)
            }
        }
    }

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
                    usize::from_str(num_str).map(|n| n * mult).map_err(|e| e.into())
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
