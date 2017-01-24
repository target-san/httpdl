#[macro_use]                // Attribute, means we're importing macros from this crate
extern crate clap;          // Declare external crate dependency
extern crate hyper;
#[macro_use]
extern crate error_chain;
// Import symbols from STDLIB or other crates
use std::fs;                // Filesystem stuff
use std::io::{self, Read, Write};  // We need this to invoke methods of Read trait
use std::path::Path;        // FS paths manipulation

// Inline nested module
mod errors {
    // Let's describe our errors
    error_chain! {
        foreign_links {
            Io(::std::io::Error);
            ParseInt(::std::num::ParseIntError);
            ArgError(::clap::Error);
            Http(::hyper::Error);
        }
    }
}
use errors::*;
// main holds in fact only error reporting routines
quick_main!(run);
// Main entry point
fn run() -> Result<()> {
    // Parse arguments 
    let args = parse_args()?;
    // Read text of list file into single string
    let file_list_text = fs::File::open(&args.list_file)
        .and_then(|mut file| {
            let mut text = String::new();
            file.read_to_string(&mut text)?;
            Ok(text)
        })
        .chain_err(|| format!("Failed to read list file {}", &args.list_file))?;
    // Construct iterator over list of URLs and file names
    let urls_list = file_list_text
        .lines()    // Iterate over lines in buffer
        .filter_map(|line| { // Maps each value to optional value. Absent values are filtered out
            // Split each line by whitespace chars, then filter out all empty pieces
            let mut pieces = line.split(|c| " \r\n\t".contains(c)).filter(|s| !s.is_empty());
            let url = pieces.next();        // Pick first piece as URL
            let filename = pieces.next();   // Pick second piece as filename
            // Next, if both 'url' and 'filename' contain actual values, pack them into a tuple and return
            if let (Some(url_value), Some(filename_value)) = (url, filename) {
                Some((url_value, filename_value))
            }
            else { None } // Otherwise, signal that there's no pair for this particular line
        })
        .fuse(); // Will guarantee that iterator will steadily return None after sequence end
    
    download_files(&args.dest_dir, urls_list);

    Ok(())
}
// Iterate through list of files and download them one by one
fn download_files<'a, I>(dir: &str, list: I) where I: Iterator<Item=(&'a str, &'a str)> {
    // Iterate list of download targets
    for (url, filename) in list {
        // Small info message, just for our convenience
        println!("Downloading: {} -> {}", url, filename);
        if let Err(error) = download_file(url, dir, filename) {
            let _ = writeln!(io::stderr(), "  Failed {} -> {}\n  Error: {}", url, filename, error);
            for inner in error.iter().skip(1) {
                let _ = writeln!(io::stderr(), "    Caused by: {}", inner);
            }
        }
    }
}
// Download exactly one file
fn download_file(url: &str, dir: &str, filename: &str) -> Result<()> {
    // Fire request to HTTP server via Hyper, obtain response
    let mut response = hyper::Client::new().get(url).send()?;
    // Check status, bail-out if not okay
    if !response.status.is_success() {
        bail!("HTTP request failed - {}", response.status);
    }
    // Lastly, create target file
    let mut file = fs::File::create(Path::new(dir).join(filename))?;
    // And copy all the stuff there form response
    let _ = io::copy(&mut response, &mut file)?;
    Ok(())
}
// Contains parsed arguments
struct Args {
    dest_dir: String,
    list_file: String,
    threads_num: usize,
    speed_limit: usize,
}
// Parse arguments from command line; any errors handled internally
fn parse_args() -> Result<Args> {
    use clap::Arg;  // We can import external symbols at any scope
    // Here, we define argument parser
    let args = app_from_crate!()
        .arg(Arg::with_name("dest_dir")
            .help("Directory where to store downloaded files")
            .short("o")
            .takes_value(true)
            .required(true)
        )
        .arg(Arg::with_name("list_file")
            .help("File which contains list of all URLs to download and local names for them")
            .short("f")
            .takes_value(true)
            .required(true)
        )
        .arg(Arg::with_name("threads_num")
            .help("Number of threads to use")
            .short("n")
            .default_value("1")
        )
        .arg(Arg::with_name("speed_limit")
            .help("Limit speed to N bytes per second; '0' means no limit
Suffixes supported:
    k, K - kilobytes (1024 bytes)
    m, M - megabytes (1024*1024 bytes)
")
            .short("l")
            .default_value("0")
        )
        .get_matches_safe()?;
    // Simply return Args structure, with all arguments parsed
    return Ok(Args {
        dest_dir:    parse_arg(&args, "dest_dir",    parse_dir)?,
        list_file:   parse_arg(&args, "list_file",   parse_file)?,
        threads_num: parse_arg(&args, "threads_num", parse_threads_num)?,
        speed_limit: parse_arg(&args, "speed_limit", parse_speed_limit)?,
    });
    // Tiny helper function which takes argument value from matches and parses it into actual value
    fn parse_arg<T, F>(args: &clap::ArgMatches, name: &str, parse: F) -> Result<T>
        where F: FnOnce(&str) -> Result<T>
    {
        // Take value of argument by name, then unwrap it from option and parse using external function
        parse(args.value_of(name).unwrap()).chain_err(|| format!("Invalid program argument <{}>", name))
    }
    // Check that specified string represents existing directory
    fn parse_dir(value: &str) -> Result<String> {
        if fs::metadata(value)?.is_dir() {
            Ok(value.to_owned())
        }
        else {
            bail!("{}: not a directory", value)
        }
    }
    // Check that string is a path to existing file
    fn parse_file(value: &str) -> Result<String> {
        if fs::metadata(value)?.is_file() {
            Ok(value.to_owned())
        }
        else {
            bail!("{}: not a file", value)
        }
    }
    // Number of threads, usize, 1 or more
    fn parse_threads_num(value: &str) -> Result<usize> {
        match value.parse()? {
            0 => bail!("cannot be zero"),
            n => Ok(n)
        }
    }
    // Speed limit, taking suffixes into account
    fn parse_speed_limit(value: &str) -> Result<usize> {
        // char_indices will iterate string slice as a sequence of pairs,
        // where first element is the byte offset of character, and the second is a Unicode character
        // last() will return last iterator in sequence
        match value.char_indices().last() {
            None => Ok(0), // Means string is empty, treat as 0
            Some((last_index, last_char)) => {
                let multiplier: usize = match last_char {
                    'k' | 'K' => 1024,
                    'm' | 'M' => 1024*1024,
                    _ => 1
                };
                // We'll parse number without suffix
                let number = if multiplier == 1 { value } else { &value[..last_index] };
                Ok(number.parse::<usize>()? * multiplier)
            }
        }
    }
}
