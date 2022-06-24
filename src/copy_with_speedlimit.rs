use std::io::{self, Read, Write};
use std::thread;
/// Modified copy of std::io::copy function
/// Differs from original by using extra parameter, speed limiter
/// This speed limiter is used on each copy operation to determine how many actual bytes
/// should be copied
pub fn copy_with_speedlimit<R: ?Sized, W: ?Sized, Q: ?Sized>(reader: &mut R, writer: &mut W, get_speed_quota: &Q) -> io::Result<u64>
    where
        R: Read,
        W: Write,
        Q: Fn(usize) -> usize
{
    let mut buf = [0; 64 * 1024];
    let mut written = 0;
    loop {
        let limit = get_speed_quota(buf.len());
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
