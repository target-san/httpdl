#[macro_use]
extern crate structopt_derive;
extern crate structopt;

#[macro_use]
extern crate error_chain;

use structopt::StructOpt;

use std::{io, fs};
use std::str::FromStr;

mod errors {
    error_chain! {
        foreign_links {
            Io(::std::io::Error);
        }
    }
}

use errors::*;
/*
quick_main!(|| {
    let matches = Args::clap().get_matches_safe()?;
    let args = Args::from_clap(matches);

    Ok(())
});
*/
fn main() {
    let args = Args::from_args();
    println!("Arguments so far: {:?}", args);
}

#[derive(Debug, StructOpt)]
#[structopt()]
struct Args {
    #[structopt(short = "o")]
    /// Destination dir where to store downloaded files
    dest_dir:    DirEntry,
    #[structopt(short = "f")]
    /// File which lists all download URLs and their respective local names
    list_file:   FileEntry,
    #[structopt(short = "n", default_value = "1")]
    /// Number of concurrent downloader threads
    threads_num: usize,
    #[structopt(short = "l", default_value = "0", help =
"Maximum download speed, in bytes per second; '0' means no limit;
Supported suffixes for value:
    k, K - kilobytes, multiples of 1024s
    m, M - megabytes, multiples of 1024*1024
")]
    speed_limit: usize,
}

#[derive(Debug)]
struct DirEntry(String);

impl FromStr for DirEntry {
    type Err = Error;
    fn from_str(path: &str) -> Result<Self> {
        if fs::metadata(path)?.is_dir() {
            Ok(DirEntry(path.to_owned()))
        }
        else { bail!("{}: not a directory", path) }
    }
}

#[derive(Debug)]
struct FileEntry(String);

impl FromStr for FileEntry {
    type Err = Error;
    fn from_str(path: &str) -> Result<Self> {
        if fs::metadata(path)?.is_file() {
            Ok(FileEntry(path.to_owned()))
        }
        else { bail!("{}: not a file", path) }
    }
}
