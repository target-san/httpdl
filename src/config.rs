use std::fs;
use std::str::FromStr;

use anyhow::{bail, Result};

use clap::Parser;

/// Contains execution parameters and provides their parsing from application's CLI arguments
#[derive(Parser, Debug)]
#[clap(author, version, about)]
pub struct Config {
    #[clap(short = 'o', value_parser = parse_dest_dir)]
    /// Destination directory where to store downloaded files
    pub dest_dir: String,
    #[clap(short = 'f', value_parser = parse_list_file_path)]
    /// File which contains list of URLs to download and local names for files
    pub list_file: String,
    #[clap(short = 'n', value_parser = parse_threads_num, default_value_t = 1)]
    /// Number of worker threads to use
    pub threads_num: usize,
    #[clap(short = 'l', value_parser = parse_speed_limit, default_value_t = 0, verbatim_doc_comment)]
    /// Global speed limit, in bytes per second. 0 means no limit
    ///
    /// Suffixes supported:
    ///     k, K - kilobytes, i.e. 1024's of bytes
    ///     m, M - megabytes, i.e. 1024*1024's of bytes
    pub speed_limit: usize,
}
/// Parses string as directory path and checks that directory actually exists
fn parse_dest_dir(arg: &str) -> Result<String> {
    if fs::metadata(arg)?.is_dir() {
        Ok(arg.to_owned())
    } else {
        bail!("{}: not a directory", arg)
    }
}
/// Parses string as file path and checks that file actually exists
fn parse_list_file_path(arg: &str) -> Result<String> {
    if fs::metadata(arg)?.is_file() {
        Ok(arg.to_owned())
    } else {
        bail!("{}: not a file", arg)
    }
}
/// Parses string as unsigned number, limits it to 1.. range
fn parse_threads_num(arg: &str) -> Result<usize> {
    let num = usize::from_str(arg)?;
    if num != 0 {
        Ok(num)
    } else {
        bail!("Expected number > 0")
    }
}
/// Parses string as number, supports multiplication suffixes for kilo (*1024) and mega (*1024*1024)
fn parse_speed_limit(arg: &str) -> Result<usize> {
    match arg.char_indices().last() {
        None => bail!("Expected number"),
        Some((last_index, last_char)) => {
            // Set multiplier based on speed limit suffix
            let mult: usize = match last_char {
                'k' | 'K' => 1024,
                'm' | 'M' => 1024 * 1024,
                _ => 1,
            };
            // Next, get actual number string based on multiplier being recognized or not
            let num_str = if mult == 1 {
                arg
            } else {
                arg.split_at(last_index).0
            };
            // We could map error, but it's also possible to use '?'
            // and simply return result wrapped into Ok
            Ok(usize::from_str(num_str).map(|n| n * mult)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use assert_matches::assert_matches;
    use clap::Parser;
    use std::env;

    // Macro which shortens matching assertion expression
    macro_rules! assert_args_match {
        ([ $($cli:expr),* ], $($arg:tt)*) => {
            assert_matches!(crate::config::Config::try_parse_from(["", $($cli),* ]), $($arg)*);
        };
    }

    #[test]
    fn parse_simple_success() {
        let existing_dir = env::current_dir().unwrap();
        let existing_file = env::current_exe().unwrap();

        let dir = existing_dir.to_str().unwrap();
        let file = existing_file.to_str().unwrap();
        // Simple parsing with both dir and file found and no optional parameters
        // should result in success with default values
        assert_args_match!(
            ["-o", dir, "-f", file],
            Ok(Config{ dest_dir, list_file, threads_num: 1, speed_limit: 0 })
                if dest_dir == dir && list_file == file
        );
    }

    #[test]
    fn required_params_failures() {
        let existing_dir = env::current_dir().unwrap();
        let nonexistent_dir = existing_dir.join("this-directory-does-not-exist");
        let existing_file = env::current_exe().unwrap();
        let nonexistent_file = existing_file.join("this-file-does-not-exist");

        let dir = existing_dir.to_str().unwrap();
        let no_dir = nonexistent_dir.to_str().unwrap();
        let file = existing_file.to_str().unwrap();
        let no_file = nonexistent_file.to_str().unwrap();
        // Missing one or both required parameters
        assert_args_match!([], Err(_));
        assert_args_match!(["-o", dir], Err(_));
        assert_args_match!(["-f", file], Err(_));
        // Check if either dir or file does not exist
        assert_args_match!(["-o", no_dir, "-f", no_file], Err(_));
        assert_args_match!(["-o", no_dir, "-f", file], Err(_));
        assert_args_match!(["-o", dir, "-f", no_file], Err(_));
    }

    #[test]
    fn threads_num_failures() {
        let existing_dir = env::current_dir().unwrap();
        let existing_file = env::current_exe().unwrap();

        let dir = existing_dir.to_str().unwrap();
        let file = existing_file.to_str().unwrap();
        // threads_num - main failure cases
        assert_args_match!(["-o", dir, "-f", file, "-n", "0"], Err(_));
        assert_args_match!(["-o", dir, "-f", file, "-n", "-1"], Err(_));
        assert_args_match!(["-o", dir, "-f", file, "-n", ""], Err(_));
        assert_args_match!(["-o", dir, "-f", file, "-n", "abc"], Err(_));
    }

    #[test]
    fn threads_num_successes() {
        let existing_dir = env::current_dir().unwrap();
        let existing_file = env::current_exe().unwrap();

        let dir = existing_dir.to_str().unwrap();
        let file = existing_file.to_str().unwrap();
        // threads_num - several success cases
        assert_args_match!(
            ["-o", dir, "-f", file, "-n", "1"],
            Ok(Config { threads_num: 1, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-n", "2"],
            Ok(Config { threads_num: 2, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-n", "4"],
            Ok(Config { threads_num: 4, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-n", "7"],
            Ok(Config { threads_num: 7, .. })
        );
    }

    #[test]
    fn speed_limit_successes() {
        let existing_dir = env::current_dir().unwrap();
        let existing_file = env::current_exe().unwrap();

        let dir = existing_dir.to_str().unwrap();
        let file = existing_file.to_str().unwrap();
        // speed_limit - simple success cases
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "0"],
            Ok(Config { speed_limit: 0, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1"],
            Ok(Config { speed_limit: 1, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1000"],
            Ok(Config {
                speed_limit: 1_000,
                ..
            })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1000000"],
            Ok(Config {
                speed_limit: 1_000_000,
                ..
            })
        );
        // speed_limit - suffix parse successes
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "0k"],
            Ok(Config { speed_limit: 0, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "0K"],
            Ok(Config { speed_limit: 0, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "0m"],
            Ok(Config { speed_limit: 0, .. })
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "0M"],
            Ok(Config { speed_limit: 0, .. })
        );

        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1k"],
            Ok(Config{ speed_limit: s, .. }) if s == 1_024
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1K"],
            Ok(Config{ speed_limit: s, .. }) if s == 1_024
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1m"],
            Ok(Config{ speed_limit: s, .. }) if s == 1_024*1_024
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "1M"],
            Ok(Config{ speed_limit: s, .. }) if s == 1_024*1_024
        );

        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "2k"],
            Ok(Config{ speed_limit: s, .. }) if s == 2*1_024
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "2K"],
            Ok(Config{ speed_limit: s, .. }) if s == 2*1_024
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "2m"],
            Ok(Config{ speed_limit: s, .. }) if s == 2*1_024*1_024
        );
        assert_args_match!(
            ["-o", dir, "-f", file, "-l", "2M"],
            Ok(Config{ speed_limit: s, .. }) if s == 2*1_024*1_024
        );
    }

    #[test]
    fn speed_limit_failures() {
        let existing_dir = env::current_dir().unwrap();
        let existing_file = env::current_exe().unwrap();

        let dir = existing_dir.to_str().unwrap();
        let file = existing_file.to_str().unwrap();
        // speed_limit - simple failure cases
        assert_args_match!(["-o", dir, "-f", file, "-l", "-1"], Err(_));
        assert_args_match!(["-o", dir, "-f", file, "-l", ""], Err(_));
        assert_args_match!(["-o", dir, "-f", file, "-l", "abc"], Err(_));
        // Check failure on unknown suffix
        assert_args_match!(["-o", dir, "-f", file, "-l", "2u"], Err(_));
    }
}
