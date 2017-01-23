#[macro_use]            // Attribute, means we're importing macros from this crate
extern crate clap;      // Declare external crate dependency
// Import symbols from STDLIB or other crates
use std::str::FromStr;  // Describes any type which can be read from string slice
use std::fmt::Debug;    // Debug types can be dumped as text, for debug purposes
use std::fs;            // Filesystem stuff
use std::io::Read;      // We need this to invoke methods of Read trait
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
    // Just for our convenience, 
    for item in urls_list {
        println!("{:?}", item);
    }
}
// Contains parsed arguments
#[derive(Debug)]
struct Args {
    dest_dir: String,
    list_file: String,
    threads_num: usize,
    speed_limit: usize,
}
// Parse arguments from command line; any errors handled internally
fn parse_args() -> Args {
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
        .get_matches(); // Will either return arguments map or interrupt program with descriptive error
    // Simply return Args structure, with all arguments parsef
    return Args {
        dest_dir:    parse_arg(&args, "dest_dir"),
        list_file:   parse_arg(&args, "list_file"),
        threads_num: parse_arg(&args, "threads_num"),
        speed_limit: parse_arg(&args, "speed_limit"),
    };
    // Tiny helper function which takes argument value from matches and parses it into actual value
    fn parse_arg<T: FromStr>(args: &clap::ArgMatches, name: &str) -> T
        where <T as FromStr>::Err: Debug // A bit of Rust traits magic
    {
        // Take value of argument by name, then unwrap it from option, then parse string slice, then unwrap result into value
        args.value_of(name).unwrap().parse().unwrap()
    }
}
