#[macro_use]            // Attribute, means we're importing macros from this crate
extern crate clap;      // Declare external crate dependency
// Import symbols from STDLIB or other crates
use std::str::FromStr;  // Describes any type which can be read from string slice
use std::fmt::Debug;    // Debug types can be dumped as text, for debug purposes
// Main entry point
fn main() {
    println!("Arguments:\n{:?}", parse_args());
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
