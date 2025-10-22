use std::io::Read;

use bytes::Bytes;

// decompress ----------------------------------------------------------

pub(crate) fn decompress(body: Bytes, size: usize, content_encoding: String) -> (Bytes, usize) {
    match content_encoding.to_ascii_lowercase().as_str() {
        "br" => {
            let mut decoder = brotli::Decompressor::new(std::io::Cursor::new(body.clone()), 4096);
            let mut decompressed_body = Vec::<u8>::new();

            if let Ok(decompressed_size) = decoder.read_to_end(&mut decompressed_body) {
                return (Bytes::from(decompressed_body), decompressed_size);
            }
        }

        "deflate" => {
            let mut decoder = flate2::read::ZlibDecoder::new(std::io::Cursor::new(body.clone()));
            let mut decompressed_body = Vec::<u8>::new();

            if let Ok(decompressed_size) = decoder.read_to_end(&mut decompressed_body) {
                return (Bytes::from(decompressed_body), decompressed_size);
            }
        }

        "gzip" => {
            let mut decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(body.clone()));
            let mut decompressed_body = Vec::<u8>::new();

            if let Ok(decompressed_size) = decoder.read_to_end(&mut decompressed_body) {
                return (Bytes::from(decompressed_body), decompressed_size);
            }
        }

        "zstd" => {
            if let Ok(mut decoder) =
                zstd::stream::read::Decoder::new(std::io::Cursor::new(body.clone()))
            {
                let mut decompressed_body = Vec::<u8>::new();

                if let Ok(decompressed_size) = decoder.read_to_end(&mut decompressed_body) {
                    return (Bytes::from(decompressed_body), decompressed_size);
                }
            };
        }

        _ => {}
    }

    (body.clone(), size)
}
