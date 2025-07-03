// Trait to abstract reading for LspMessageStream
pub trait LspReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}

// Implement LspReader for BufReader wrapping Box<dyn LspStream>
impl LspReader for std::io::BufReader<Box<dyn crate::lsp_client::LspStream>> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.get_mut().read(buf)
    }
}

#[allow(dead_code)]
pub struct LspMessageStream<R: LspReader> {
    reader: R,
    message_buf: Vec<u8>,
    content_length: Option<usize>,
    is_content_length: bool,
}

#[allow(dead_code)]
impl<R: LspReader> LspMessageStream<R> {
    pub fn new(reader: R) -> Self {
        LspMessageStream {
            reader,
            message_buf: Vec::new(),
            content_length: None,
            is_content_length: false,
        }
    }

    pub fn message(&self) -> String {
        String::from_utf8_lossy(&self.message_buf).to_string()
    }

    fn next_byte(&mut self) -> Result<u8, String> {
        let mut bs = [0u8; 1];
        match self.reader.read(&mut bs) {
            Ok(0) => Err("Input stream closed".to_string()),
            Ok(1) => {
                let b: u8 = bs[0];
                self.message_buf.push(b);
                Ok(b)
            }
            Ok(n) => Err(format!(
                "Expected no more than 1 byte to be read, but received: {}",
                n
            )),
            Err(e) => Err(format!("Error reading byte: {}", e)),
        }
    }

    pub fn next_payload(&mut self) -> Result<String, String> {
        self.message_buf.clear();
        self.content_length = None;
        self.parse_header_name()
    }

    fn position(&self) -> usize {
        self.message_buf.len()
    }

    fn escape(b: u8) -> Result<String, String> {
        match b {
            b'\n' => Ok("\\n".to_string()),
            b'\t' => Ok("\\t".to_string()),
            b'\x08' => Ok("\\b".to_string()),
            b'\r' => Ok("\\r".to_string()),
            b'\x0c' => Ok("\\f".to_string()),
            _ => std::str::from_utf8(&[b])
                .map(|s| s.to_string())
                .map_err(|e| format!("Failed to convert byte to UTF-8: {}", e)),
        }
    }

    fn parse_header_name(&mut self) -> Result<String, String> {
        let start = self.position();
        loop {
            let b = self.next_byte()?;
            match b {
                b'\r' => {
                    let c = self.next_byte()?;
                    if c != b'\n' {
                        return Err(format!(
                            "Expected \\r to be followed by \\n, not '{:?}':\n{}",
                            Self::escape(c),
                            self.message()
                        ));
                    }
                    let line_length = self.position() - start - 2;
                    if line_length == 0 {
                        if let Some(_) = self.content_length {
                            return self.parse_body();
                        }
                        return Err(
                            format!(
                                "Reached end of header section without defining the Content-Length:\n{}",
                                self.message()
                            )
                        );
                    }
                    return Err(format!(
                        "Reached out-of-sequence carriage-return while parsing header name:\n{}",
                        self.message()
                    ));
                }
                b'\n' => {
                    return Err(format!(
                        "Reached out-of-sequence newline while parsing header name:\n{}",
                        self.message()
                    ))
                }
                b':' => {
                    let stop = self.position() - 1;
                    let header_name = std::str::from_utf8(&self.message_buf[start..stop]).map_err(
                        |e| format!("Failed to convert header bytes to a UTF-8 string: {}", e),
                    )?;
                    self.is_content_length = header_name.to_uppercase() == "CONTENT-LENGTH";
                    return self.parse_header_value();
                }
                _ => continue,
            }
        }
    }

    fn drop_whitespace(&mut self) -> Result<u8, String> {
        let mut b = self.next_byte()?;
        while (b == b' ') || (b == b'\t') {
            b = self.next_byte()?;
        }
        return Ok(b);
    }

    fn parse_header_value(&mut self) -> Result<String, String> {
        let mut b = self.drop_whitespace()?;
        let start = self.position() - 1;
        loop {
            match b {
                b'\r' => {
                    let c = self.next_byte()?;
                    if c != b'\n' {
                        return Err(format!(
                            "Expected \\r to be followed by \\n, not '{:?}':\n{}",
                            Self::escape(c),
                            self.message()
                        ));
                    }
                    if self.is_content_length {
                        let end = self.position() - 2;
                        let header_value = &self.message_buf[start..end];
                        if header_value.is_empty() {
                            return Err("Header `Content-Length` has no value!".to_string());
                        }
                        let content_length = std::str::from_utf8(header_value)
                            .map_err(|e| format!(
                                "Invalid UTF-8 character at byte {}:\n{}",
                                start + e.valid_up_to(),
                                self.message()
                            ))?
                            .parse::<usize>()
                            .map_err(|_| {
                                format!(
                                    "Invalid digit in value: {:?}",
                                    std::str::from_utf8(header_value)
                                )
                            })?;
                        self.content_length = Some(content_length);
                    }
                    return self.parse_header_name();
                }
                b'\n' => {
                    return Err(format!(
                        "Reached out-of-sequence newline while parsing header name:\n{}",
                        self.message()
                    ))
                }
                _ => b = self.next_byte()?,
            }
        }
    }

    fn parse_body(&mut self) -> Result<String, String> {
        if let Some(content_length) = self.content_length {
            let start = self.position();
            let stop = self.position() + content_length;
            self.message_buf.resize(stop, 0);
            let mut bytes_read = 0;
            while bytes_read < content_length {
                bytes_read += self.reader
                    .read(&mut self.message_buf[start..stop])
                    .map_err(|e| format!("Error reading message body: {}", e))?;
            }
            let message = std::str::from_utf8(&self.message_buf[start..stop]).map_err(|e| {
                format!(
                    "Failed to parse body from bytes: {:?}: {}",
                    &self.message_buf[start..stop],
                    e
                )
            })?;
            Ok(message.to_string())
        } else {
            Err("Cannot parse body before establishing the number of bytes.".to_string())
        }
    }
}
