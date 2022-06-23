use std::fs;
use std::process::exit;
use std::str::FromStr;

use anyhow::{bail, Context, Result};

// Pre-parsed arguments received from command line
pub struct Config
{
    pub dest_dir:    String,
    pub list_file:   String,
    pub threads_num: usize,
    pub speed_limit: usize,
}

// Parse command line arguments
pub fn parse_args() -> Config {
    // Import errors definitions related to parsing arguments
    use clap::Arg;
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
            eprintln!("Error: {}", err);
            for e in err.chain().skip(1) {
                eprintln!("  Caused by: {}", e);
            }
            eprintln!("{}", args.usage());
            exit(1);
        }
    };
    // Simply construct Args and return any error if one occurs
    fn process_args(args: &clap::ArgMatches) -> Result<Config> {
        Ok(Config {
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
        func(args.value_of(name).unwrap())
            .with_context(|| format!("Error parsing command line argument <{}>", name))
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
