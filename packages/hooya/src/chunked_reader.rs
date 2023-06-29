use std::io::{self, Read};

pub struct ChunkedReader<R> {
    reader: R,
    chunk_size: usize,
}

impl<R> ChunkedReader<R> {
    pub fn new(r: R) -> Self {
        ChunkedReader {
            reader: r,
            chunk_size: 1024 * 1024, // Read in 1MiB chunks
        }
    }
}

impl<R: Read> Iterator for ChunkedReader<R> {
    type Item = io::Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut buffer = vec![0u8; self.chunk_size];
        match self.reader.read(&mut buffer) {
            Ok(0) => None,
            Ok(n) => Some(Ok(buffer[..n].to_vec())),
            Err(e) => Some(Err(e)),
        }
    }
}
