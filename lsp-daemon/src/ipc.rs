use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

// Re-export platform-specific types
#[cfg(unix)]
pub use unix_impl::{IpcListener, IpcStream};

#[cfg(windows)]
pub use windows_impl::{IpcListener, IpcStream};

/// Trait for platform-agnostic IPC listener
#[async_trait]
pub trait IpcListenerTrait: Send + Sync {
    type Stream: IpcStreamTrait;

    async fn accept(&self) -> Result<Self::Stream>;
    fn local_addr(&self) -> Result<String>;
}

/// Trait for platform-agnostic IPC stream
pub trait IpcStreamTrait: AsyncRead + AsyncWrite + Send + Sync + Unpin {
    fn peer_addr(&self) -> Result<String>;
}

// Unix implementation
#[cfg(unix)]
mod unix_impl {
    use super::*;
    use std::path::Path;
    use tokio::net::{UnixListener as TokioUnixListener, UnixStream as TokioUnixStream};

    pub struct IpcListener {
        listener: TokioUnixListener,
        path: String,
    }

    impl IpcListener {
        pub async fn bind(path: &str) -> Result<Self> {
            // Remove existing socket file if it exists
            if Path::new(path).exists() {
                std::fs::remove_file(path)?;
            }

            // Create parent directory if needed
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }

            let listener = TokioUnixListener::bind(path)?;
            Ok(Self {
                listener,
                path: path.to_string(),
            })
        }

        pub async fn accept(&self) -> Result<IpcStream> {
            let (stream, _) = self.listener.accept().await?;
            Ok(IpcStream { stream })
        }

        pub fn local_addr(&self) -> Result<String> {
            Ok(self.path.clone())
        }
    }

    impl Drop for IpcListener {
        fn drop(&mut self) {
            // Clean up socket file
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub struct IpcStream {
        stream: TokioUnixStream,
    }

    impl IpcStream {
        pub async fn connect(path: &str) -> Result<Self> {
            let stream = TokioUnixStream::connect(path).await?;
            Ok(Self { stream })
        }

        pub fn peer_addr(&self) -> Result<String> {
            Ok("unix-peer".to_string()) // Unix sockets don't have traditional addresses
        }
    }

    impl AsyncRead for IpcStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for IpcStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.stream).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.stream).poll_shutdown(cx)
        }
    }

    impl IpcStreamTrait for IpcStream {
        fn peer_addr(&self) -> Result<String> {
            self.peer_addr()
        }
    }
}

// Windows implementation
#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::sync::Arc;
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use tokio::sync::Mutex;

    pub struct IpcListener {
        path: String,
        current_server: Arc<Mutex<Option<NamedPipeServer>>>,
    }

    impl IpcListener {
        pub async fn bind(path: &str) -> Result<Self> {
            let server_options = ServerOptions::new()
                .first_pipe_instance(true)
                .in_buffer_size(65536)
                .out_buffer_size(65536);

            // Create the first server instance
            let server = server_options.create(path)?;

            Ok(Self {
                path: path.to_string(),
                current_server: Arc::new(Mutex::new(Some(server))),
            })
        }

        pub async fn accept(&self) -> Result<IpcStream> {
            let mut server_guard = self.current_server.lock().await;

            if let Some(mut server) = server_guard.take() {
                // Wait for a client to connect
                server.connect().await?;

                // Create a new server instance for the next connection
                let server_options = ServerOptions::new()
                    .first_pipe_instance(false)
                    .in_buffer_size(65536)
                    .out_buffer_size(65536);
                let new_server = server_options.create(&self.path)?;
                *server_guard = Some(new_server);

                // Return the connected server as a stream
                // Windows named pipes work bidirectionally, so the server pipe
                // can be used for both reading and writing after connection
                Ok(IpcStream {
                    stream: IpcStreamInner::Server(server),
                })
            } else {
                Err(anyhow::anyhow!("No server available"))
            }
        }

        pub fn local_addr(&self) -> Result<String> {
            Ok(self.path.clone())
        }
    }

    enum IpcStreamInner {
        Client(NamedPipeClient),
        Server(NamedPipeServer),
    }

    pub struct IpcStream {
        stream: IpcStreamInner,
    }

    impl IpcStream {
        pub async fn connect(path: &str) -> Result<Self> {
            let client = ClientOptions::new().open(path)?;

            Ok(Self {
                stream: IpcStreamInner::Client(client),
            })
        }

        pub fn peer_addr(&self) -> Result<String> {
            Ok("windows-pipe-peer".to_string())
        }
    }

    impl AsyncRead for IpcStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            match &mut self.stream {
                IpcStreamInner::Client(client) => Pin::new(client).poll_read(cx, buf),
                IpcStreamInner::Server(server) => Pin::new(server).poll_read(cx, buf),
            }
        }
    }

    impl AsyncWrite for IpcStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            match &mut self.stream {
                IpcStreamInner::Client(client) => Pin::new(client).poll_write(cx, buf),
                IpcStreamInner::Server(server) => Pin::new(server).poll_write(cx, buf),
            }
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            match &mut self.stream {
                IpcStreamInner::Client(client) => Pin::new(client).poll_flush(cx),
                IpcStreamInner::Server(server) => Pin::new(server).poll_flush(cx),
            }
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            // Named pipes don't have a shutdown method, so we just flush
            self.poll_flush(cx)
        }
    }

    impl IpcStreamTrait for IpcStream {
        fn peer_addr(&self) -> Result<String> {
            self.peer_addr()
        }
    }
}

/// Helper function to create an IPC listener
pub async fn bind(path: &str) -> Result<IpcListener> {
    IpcListener::bind(path).await
}

/// Helper function to connect to an IPC endpoint
pub async fn connect(path: &str) -> Result<IpcStream> {
    IpcStream::connect(path).await
}
