use std::io::{self, Read, Seek, SeekFrom};

pub struct ChunkedReader<R> {
    reader: R,
    chunk_size: usize,
    remaining_bytes: Option<u64>,
}

impl<R> ChunkedReader<R> {
    pub fn new(r: R) -> Self {
        ChunkedReader {
            reader: r,
            chunk_size: 1024 * 1024, // Read in 1MiB chunks
            remaining_bytes: None,
        }
    }
}

impl<R: Read + Seek> ChunkedReader<R> {
    pub fn new_with_range(
        mut r: R,
        start: u64,
        end: Option<u64>,
    ) -> io::Result<Self> {
        r.seek(SeekFrom::Start(start))?;
        let remaining = end.map(|e| e.saturating_sub(start).saturating_add(1));
        Ok(ChunkedReader {
            reader: r,
            chunk_size: 1024 * 1024,
            remaining_bytes: remaining,
        })
    }
}

impl<R: Read> Iterator for ChunkedReader<R> {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        // check if we've reached the end of our range
        if let Some(remaining) = self.remaining_bytes {
            if remaining == 0 {
                return None;
            }
        }

        // determine how much to read this iteration
        let read_size = match self.remaining_bytes {
            Some(remaining) => {
                std::cmp::min(self.chunk_size as u64, remaining) as usize
            }
            None => self.chunk_size,
        };

        let mut buffer = vec![0u8; read_size];
        match self.reader.read(&mut buffer) {
            Ok(0) => None,
            Ok(n) => {
                // update remaining bytes if we're tracking them
                if let Some(ref mut remaining) = self.remaining_bytes {
                    *remaining = remaining.saturating_sub(n as u64);
                }
                Some(Ok(buffer[..n].to_vec()))
            }
            Err(e) => Some(Err(e)),
        }
    }
}
