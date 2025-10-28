use std::io::{self, BufReader, Empty, Read, Write};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};
use futures::{SinkExt, StreamExt};

use tracing::{error, info, trace};

pub trait LspStream: Send + 'static {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "read not supported",
        ))
    }
    fn write_all(&mut self, _buf: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "write not supported",
        ))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for Box<dyn LspStream> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.as_mut().read(buf)
    }
}

impl LspStream for std::process::ChildStdin {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        Write::write_all(self, buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        Write::flush(self)
    }
}

impl LspStream for std::process::ChildStdout {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(self, buf)
    }
}

impl LspStream for std::process::ChildStderr {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(self, buf)
    }
}

impl LspStream for std::io::PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(self, buf)
    }
}

impl LspStream for BufReader<Empty> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        Read::read(self, buf)
    }
}

pub struct AsyncLspReadStream<T: AsyncRead + Unpin + Send + 'static> {
    stream: T,
    runtime_handle: tokio::runtime::Handle,
}

impl<T: AsyncRead + Unpin + Send + 'static> AsyncLspReadStream<T> {
    pub fn new(stream: T, runtime_handle: tokio::runtime::Handle) -> Self {
        AsyncLspReadStream {
            stream,
            runtime_handle,
        }
    }
}

impl<T: AsyncRead + Unpin + Send + 'static> LspStream for AsyncLspReadStream<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.runtime_handle.block_on(AsyncReadExt::read(&mut self.stream, buf))
    }
}

pub struct AsyncLspWriteStream<T: AsyncWrite + Unpin + Send + 'static> {
    stream: Arc<Mutex<T>>,
    runtime_handle: tokio::runtime::Handle,
}

impl<T: AsyncWrite + Unpin + Send + 'static> AsyncLspWriteStream<T> {
    pub fn new(stream: Arc<Mutex<T>>, runtime_handle: tokio::runtime::Handle) -> Self {
        AsyncLspWriteStream {
            stream,
            runtime_handle,
        }
    }
}

impl<T: AsyncWrite + Unpin + Send + 'static> LspStream for AsyncLspWriteStream<T> {
    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Mutex lock failed: {}", e)))?;
        self.runtime_handle
            .block_on(AsyncWriteExt::write_all(&mut *stream, buf))
    }
    fn flush(&mut self) -> io::Result<()> {
        let mut stream = self
            .stream
            .lock()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Mutex lock failed: {}", e)))?;
        self.runtime_handle
            .block_on(AsyncWriteExt::flush(&mut *stream))
    }
}

pub struct WebSocketStreamAdapter {
    sink: Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    stream: Arc<Mutex<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
    runtime_handle: tokio::runtime::Handle,
    read_buffer: Vec<u8>,
}

impl WebSocketStreamAdapter {
    pub fn new(
        sink: Arc<Mutex<futures_util::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
        stream: Arc<Mutex<futures_util::stream::SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
        runtime_handle: tokio::runtime::Handle,
    ) -> Self {
        WebSocketStreamAdapter {
            sink,
            stream,
            runtime_handle,
            read_buffer: Vec::new(),
        }
    }
}

impl LspStream for WebSocketStreamAdapter {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.read_buffer.is_empty() {
            let to_copy = std::cmp::min(buf.len(), self.read_buffer.len());
            buf[..to_copy].copy_from_slice(&self.read_buffer[..to_copy]);
            self.read_buffer.drain(..to_copy);
            trace!("Read {} bytes from buffer", to_copy);
            return Ok(to_copy);
        }

        let result = self.runtime_handle.block_on(async {
            let mut stream = self
                .stream
                .lock()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Mutex lock failed: {}", e)))?;
            match stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    trace!("Client received WebSocket text message: {}", text);
                    Ok(text.into_bytes())
                }
                Some(Ok(Message::Binary(data))) => {
                    trace!("Client received WebSocket binary message: {:?}", data);
                    Ok(data)
                }
                Some(Ok(Message::Ping(_))) => {
                    trace!("Received Ping");
                    Ok(Vec::new())
                }
                Some(Ok(Message::Pong(_))) => {
                    trace!("Received Pong");
                    Ok(Vec::new())
                }
                Some(Ok(Message::Close(_))) => {
                    info!("WebSocket connection closed");
                    Ok(Vec::new())
                }
                Some(Ok(Message::Frame(_))) => {
                    trace!("Received Frame");
                    Ok(Vec::new())
                }
                Some(Err(e)) => {
                    error!("WebSocket error: {}", e);
                    Err(io::Error::new(io::ErrorKind::Other, e))
                }
                None => {
                    info!("WebSocket stream ended");
                    Ok(Vec::new())
                }
            }
        })?;

        if result.is_empty() {
            return Ok(0);
        }

        self.read_buffer.extend_from_slice(&result);
        let to_copy = std::cmp::min(buf.len(), self.read_buffer.len());
        buf[..to_copy].copy_from_slice(&self.read_buffer[..to_copy]);
        self.read_buffer.drain(..to_copy);
        trace!("Read {} bytes from stream", to_copy);
        Ok(to_copy)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        trace!("Client preparing to send WebSocket binary message: {:?}", buf);
        self.runtime_handle.block_on(async {
            let mut sink = self
                .sink
                .lock()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Mutex lock failed: {}", e)))?;
            sink.send(Message::Binary(buf.to_vec()))
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            sink.flush()
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(())
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.runtime_handle.block_on(async {
            let mut sink = self
                .sink
                .lock()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("Mutex lock failed: {}", e)))?;
            sink.flush()
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(())
        })
    }
}
