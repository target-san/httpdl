use std::io::{self, Read, Write};
use std::thread;
/// Copies data from reader object to writer object using speed limiter
/// 
/// # Arguments
/// 
/// * reader - source byte reader
/// * writer - destination byte writer
/// * get_speed_quota - function which receives wanted number
///     of bytes to read and returns actual number of bytes function is alowed to read;
///     number of bytes to read is hard-capped by internal buffer length
/// 
/// Implemented as modified copy of [std::io::copy] function which copies data
/// by repeatedly reading from reader object to stack buffer and then writing that same data
/// to writer. This particular version introduces additional parameter, speed limiter function,
/// which tells on each copy iteration how much data is actually allowed to copy
pub fn copy_with_speedlimit<R: ?Sized, W: ?Sized, Q: ?Sized>(reader: &mut R, writer: &mut W, get_speed_quota: &Q) -> io::Result<u64>
    where
        R: Read,
        W: Write,
        Q: Fn(usize) -> usize
{
    let mut buf = [0; 64 * 1024];
    let mut written = 0;
    loop {
        let limit = get_speed_quota(buf.len()).min(buf.len());
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

#[cfg(test)]
mod tests {
    //! TODO: consider fuzzing Read and Write to write some failing tests

    use assert_matches::assert_matches;
    use rand::{thread_rng, Rng, RngCore};
    use std::io::{Read, Write, Cursor};
    use super::copy_with_speedlimit;
    /// Stub writer which dumps input data to void yet checks that that data matches
    /// data from internal Read object
    struct ValidatingWriter<T> where T: Read {
        reader: T
    }

    impl<'a, T> Write for ValidatingWriter<T> where T: Read {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut read_buf = vec![0u8; buf.len()];
            let _ = self.reader.read(&mut read_buf)?;
            assert_eq!(&read_buf, buf);
            Ok(buf.len())
        }
    
        fn flush(&mut self) -> std::io::Result<()> {
            // Data dumped to void, so we do nothing
            Ok(())
        }
    }

    fn unlimited(amount: usize) -> usize{
        amount
    }
    
    fn simple_limit_16(amount: usize) -> usize {
        amount.min(16)
    }

    fn random_limit(amount: usize) -> usize {
        thread_rng().gen_range(0..=amount)
    }

    #[test]
    fn simple_copy() {
        let speed_funcs = [ unlimited, simple_limit_16, random_limit ];
        let samples: &[&[u8]] = &[
            b"",
            b"abcde",
            b"A quick brown fox jumps over the lazy dog",
            // TODO: deduce and add more patterns
        ];

        for speed in speed_funcs {
            // use fixed samples
            for sample in samples {
                let mut source = Cursor::new(*sample);
                let mut sink = ValidatingWriter { reader: Cursor::new(*sample) };

                assert_matches!(
                    copy_with_speedlimit(&mut source, &mut sink, &speed),
                    Ok(len) if len == sample.len() as u64
                );
            }
            // use randomized samples
            for _ in 0..32 {
                let mut sample = vec![0u8; thread_rng().gen_range(1_024..(128*1_024))];
                thread_rng().fill_bytes(&mut sample);

                let mut source = Cursor::new(&sample);
                let mut sink = ValidatingWriter { reader: Cursor::new(&sample) };

                assert_matches!(
                    copy_with_speedlimit(&mut source, &mut sink, &speed),
                    Ok(len) if len == sample.len() as u64
                );
            }
        }
    }
}
