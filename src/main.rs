#[macro_use]                // Attribute, means we're importing macros from this crate
extern crate clap;          // Declare external crate dependency
extern crate hyper;
#[macro_use]
extern crate error_chain;
// Import symbols from STDLIB or other crates
use std::fs;                // Filesystem stuff
use std::io::{self, Read, Write};  // We need this to invoke methods of Read trait
use std::path::Path;        // FS paths manipulation
use std::process::exit;

use hyper::Client;
use hyper::status::StatusCode;

// Main entry point
fn main() {
    // Parse arguments 
    let args = parse_args();
    // Read text of list file into single string
    let file_list_text = {
        let mut file = fs::File::open(args.list_file).unwrap(); // Open file
        let mut text = String::new();                           // Create buffer for file text
        file.read_to_string(&mut text).unwrap();                // Read file text into buffer
        text                                                    // Return buffer from block
    };
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
        .fuse();
    // Iterate list of download targets
    for (url, filename) in urls_list {
        // Small info message, just for our convenience
        println!("Downloading: {} -> {}", url, filename);
        // Fire request to HTTP server via Hyper, obtain response
        let mut response = Client::new().get(url).send().unwrap();
        // Check status, barf if it's not okay and skip downloading
        if response.status != StatusCode::Ok {
            let _ = writeln!(io::stderr(), "    HTTP request failed: {}", response.status);
            continue;
        }
        // Lastly, create target file
        let mut file = fs::File::create(Path::new(&args.dest_dir).join(filename)).unwrap();
        // And copy all the stuff there form response
        let _ = io::copy(&mut response, &mut file).unwrap();
    }
}
// Contains parsed arguments
struct Args {
    dest_dir: String,
    list_file: String,
    threads_num: usize,
    speed_limit: usize,
}
// Inline nested module
mod parse_args_errors {
    // Let's describe our errors
    error_chain! {
    }
}
// Parse arguments from command line; any errors handled internally
fn parse_args() -> Args {
    use clap::Arg;  // We can import external symbols at any scope
    use parse_args_errors::*;
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
        .get_matches(); // Will either return arguments map or interrupt program with descriptive error

    match parse_args_inner(&args) {
        Ok(args) => args,
        Err(error) => {
            let _ = writeln!(io::stderr(), "Error: {}", error);
            for inner in error.iter().skip(1) {
                let _ = writeln!(io::stderr(), "  Caused by: {}", inner);
            }
            let _ = writeln!(io::stderr(), "{}", args.usage());
            exit(1)
        },
    }
    
    fn parse_args_inner(args: &clap::ArgMatches) -> Result<Args> {
        // Simply return Args structure, with all arguments parsef
        Ok(Args {
            dest_dir:    parse_arg(&args, "dest_dir",    parse_dir)?,
            list_file:   parse_arg(&args, "list_file",   parse_file)?,
            threads_num: parse_arg(&args, "threads_num", parse_threads_num)?,
            speed_limit: parse_arg(&args, "speed_limit", parse_speed_limit)?,
        })
    }
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
                Ok(number.parse()? * multiplier)
            }
        }
    }
}
