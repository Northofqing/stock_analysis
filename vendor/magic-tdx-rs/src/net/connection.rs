use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

use crate::error::{Result, TdxError};

trait ConnectionStream: Read + Write + Send {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    fn shutdown(&self, how: Shutdown) -> std::io::Result<()>;
    fn peer_addr(&self) -> std::io::Result<SocketAddr>;
}

impl ConnectionStream for TcpStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_read_timeout(self, timeout)
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_write_timeout(self, timeout)
    }

    fn shutdown(&self, how: Shutdown) -> std::io::Result<()> {
        TcpStream::shutdown(self, how)
    }

    fn peer_addr(&self) -> std::io::Result<SocketAddr> {
        TcpStream::peer_addr(self)
    }
}

pub struct TcpConnection {
    stream: Box<dyn ConnectionStream>,
}

impl TcpConnection {
    pub fn connect(ip: &str, port: u16, timeout_secs: f64) -> Result<Self> {
        Self::connect_with(ip, port, timeout_secs, |address, timeout| {
            TcpStream::connect_timeout(&address, timeout)
                .map(|stream| Box::new(stream) as Box<dyn ConnectionStream>)
        })
    }

    fn connect_with<Connect>(
        ip: &str,
        port: u16,
        timeout_secs: f64,
        connect: Connect,
    ) -> Result<Self>
    where
        Connect: FnOnce(SocketAddr, Duration) -> std::io::Result<Box<dyn ConnectionStream>>,
    {
        if !timeout_secs.is_finite() || timeout_secs <= 0.0 {
            return Err(TdxError::Connection(
                "timeout must be a positive finite number of seconds".into(),
            ));
        }
        let addr = format!("{}:{}", ip, port);
        let socket_addr = addr.parse::<SocketAddr>().map_err(|error| {
            TdxError::Connection(format!("invalid TDX server address {addr}: {error}"))
        })?;
        let timeout = Duration::from_secs_f64(timeout_secs);
        let stream = connect(socket_addr, timeout)
            .map_err(|e| TdxError::Connection(format!("Failed to connect to {}: {}", addr, e)))?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|e| TdxError::Connection(format!("set_read_timeout: {}", e)))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|e| TdxError::Connection(format!("set_write_timeout: {}", e)))?;
        Ok(Self { stream })
    }

    pub fn send(&mut self, data: &[u8]) -> Result<()> {
        self.stream
            .write_all(data)
            .map_err(|e| TdxError::Connection(format!("send failed: {}", e)))?;
        Ok(())
    }

    /// Read exactly `len` bytes, looping until all received or error.
    pub fn recv(&mut self, len: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; len];
        let mut total = 0;
        while total < len {
            let n = self
                .stream
                .read(&mut buf[total..])
                .map_err(|e| TdxError::Connection(format!("recv failed: {}", e)))?;
            if n == 0 {
                return Err(TdxError::Disconnected);
            }
            total += n;
        }
        Ok(buf)
    }

    pub fn close(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    pub fn is_open(&self) -> bool {
        self.stream.peer_addr().is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionStream, TcpConnection};
    use std::io::{self, Read, Write};
    use std::net::{Shutdown, SocketAddr};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Debug, Default)]
    struct StreamState {
        read_timeout: Option<Duration>,
        write_timeout: Option<Duration>,
        written: Vec<u8>,
        shutdown: bool,
    }

    struct MemoryStream {
        state: Arc<Mutex<StreamState>>,
        response: &'static [u8],
        offset: usize,
    }

    impl Read for MemoryStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let remaining = &self.response[self.offset..];
            let size = remaining.len().min(buffer.len());
            buffer[..size].copy_from_slice(&remaining[..size]);
            self.offset += size;
            Ok(size)
        }
    }

    impl Write for MemoryStream {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.state.lock().unwrap().written.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl ConnectionStream for MemoryStream {
        fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.state.lock().unwrap().read_timeout = timeout;
            Ok(())
        }

        fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
            self.state.lock().unwrap().write_timeout = timeout;
            Ok(())
        }

        fn shutdown(&self, _how: Shutdown) -> io::Result<()> {
            self.state.lock().unwrap().shutdown = true;
            Ok(())
        }

        fn peer_addr(&self) -> io::Result<SocketAddr> {
            Ok("127.0.0.1:7709".parse().unwrap())
        }
    }

    #[test]
    fn injected_transport_preserves_connection_error_and_timeout() {
        let result = TcpConnection::connect_with("127.0.0.1", 7709, 0.02, |address, timeout| {
            assert_eq!(address, "127.0.0.1:7709".parse::<SocketAddr>().unwrap());
            assert_eq!(timeout, Duration::from_millis(20));
            Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "deterministic refusal",
            ))
        });
        let error = match result {
            Ok(_) => panic!("connection unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("Failed to connect"));
        assert!(error.to_string().contains("deterministic refusal"));
    }

    #[test]
    fn injected_stream_uses_timeouts_and_public_io_contract() {
        let state = Arc::new(Mutex::new(StreamState::default()));
        let stream_state = Arc::clone(&state);
        let mut connection =
            TcpConnection::connect_with("127.0.0.1", 7709, 0.1, move |_address, _timeout| {
                Ok(Box::new(MemoryStream {
                    state: stream_state,
                    response: b"pong",
                    offset: 0,
                }))
            })
            .unwrap();

        assert!(connection.is_open());
        connection.send(b"ping").unwrap();
        assert_eq!(connection.recv(4).unwrap(), b"pong");
        connection.close();

        let state = state.lock().unwrap();
        assert_eq!(state.read_timeout, Some(Duration::from_millis(100)));
        assert_eq!(state.write_timeout, Some(Duration::from_millis(100)));
        assert_eq!(state.written, b"ping");
        assert!(state.shutdown);
    }
}
