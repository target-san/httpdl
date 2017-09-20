#[macro_use]
extern crate structopt_derive;
extern crate structopt;

use structopt::StructOpt;

fn main() {
    let args = Args::from_args();
    println!("Arguments so far: {:?}", args);
}

#[derive(Debug, StructOpt)]
#[structopt()]
struct Args {
    #[structopt(short = "o")]
    /// Destination dir where to store downloaded files
    dest_dir: String,
    #[structopt(short = "f")]
    /// File which lists all download URLs and their respective local names
    list_file: String,
    #[structopt(short = "n", default_value = "1")]
    /// Number of concurrent downloader threads
    threads_num: usize,
    #[structopt(short = "l", default_value = "0")]
    /// Maximum download speed, in bytes per second; '0' means no limit
    /// Supported suffixes for value:
    ///     k, K - kilobytes, multiples of 1024s
    ///     m, M - megabytes, multiples of 1024*1024
    speed_limit: usize,
}
