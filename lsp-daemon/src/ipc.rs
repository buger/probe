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
    use fs2::FileExt;
    use std::fs::{File, OpenOptions};
    use std::path::Path;
    use std::time::Duration;
    use tokio::net::{UnixListener as TokioUnixListener, UnixStream as TokioUnixStream};

    pub struct IpcListener {
        listener: TokioUnixListener,
        path: String,
        _lock_file: Option<File>, // Keep lock file open to maintain the lock
    }

    impl IpcListener {
        pub async fn bind(path: &str) -> Result<Self> {
            // Use a lock file to coordinate socket binding across multiple processes
            let lock_path = format!("{path}.bind.lock");
            let lock_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&lock_path)
                .map_err(|e| anyhow::anyhow!("Failed to open socket bind lock file: {}", e))?;

            // Acquire exclusive lock for the socket binding operation
            lock_file.try_lock_exclusive().map_err(|_| {
                anyhow::anyhow!("Another process is currently binding to socket {}", path)
            })?;

            // Now we have exclusive access to check and bind the socket
            let result = Self::bind_internal(path, lock_file).await;

            // The lock will be released when the lock_file is dropped (either on success or error)
            result
        }

        async fn bind_internal(path: &str, lock_file: File) -> Result<Self> {
            // Check if socket file exists and if a daemon is listening
            if Path::new(path).exists() {
                // Try to connect to see if a daemon is actually running
                match TokioUnixStream::connect(path).await {
                    Ok(_) => {
                        // Another daemon is running on this socket
                        return Err(anyhow::anyhow!(
                            "Socket {} is already in use by another daemon",
                            path
                        ));
                    }
                    Err(_) => {
                        // Socket file exists but no daemon is listening (stale socket)
                        tracing::info!("Removing stale socket file: {}", path);
                        std::fs::remove_file(path)?;
                    }
                }
            }

            // Create parent directory if needed
            if let Some(parent) = Path::new(path).parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Bind the socket - this is now protected by our exclusive lock
            let listener = match TokioUnixListener::bind(path) {
                Ok(l) => l,
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                    // This shouldn't happen with our locking, but handle it gracefully
                    tracing::warn!(
                        "Socket bind failed due to address in use, retrying after delay"
                    );
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    TokioUnixListener::bind(path)?
                }
                Err(e) => return Err(e.into()),
            };

            Ok(Self {
                listener,
                path: path.to_string(),
                _lock_file: Some(lock_file), // Keep the lock file open
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
            // Release the lock file first
            if let Some(lock_file) = self._lock_file.take() {
                let _ = FileExt::unlock(&lock_file);
                drop(lock_file);
                // Clean up the lock file
                let lock_path = format!("{}.bind.lock", self.path);
                let _ = std::fs::remove_file(&lock_path);
            }

            // Clean up socket file
            if let Err(e) = std::fs::remove_file(&self.path) {
                // Only log at trace level since this is cleanup code and the file might not exist
                tracing::trace!("Failed to remove socket file during cleanup {}: {} (this is usually not a problem)", self.path, e);
            } else {
                tracing::trace!("Successfully cleaned up socket file: {}", self.path);
            }
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
    use tracing;

    pub struct IpcListener {
        path: String,
        current_server: Arc<Mutex<Option<NamedPipeServer>>>,
    }

    impl IpcListener {
        pub async fn bind(path: &str) -> Result<Self> {
            // Create the first server instance
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .in_buffer_size(65536)
                .out_buffer_size(65536)
                .create(path)?;

            Ok(Self {
                path: path.to_string(),
                current_server: Arc::new(Mutex::new(Some(server))),
            })
        }
    }

    impl Drop for IpcListener {
        fn drop(&mut self) {
            // Log cleanup action
            tracing::debug!("Cleaning up Windows named pipe: {}", self.path);

            // Named pipes on Windows are automatically cleaned up when the last handle is closed
            // The Tokio NamedPipeServer will handle the cleanup when it's dropped
            // We just need to ensure any remaining server instance is dropped
            if let Ok(mut server_guard) = self.current_server.try_lock() {
                if server_guard.take().is_some() {
                    tracing::debug!(
                        "Closed remaining named pipe server instance for: {}",
                        self.path
                    );
                }
            } else {
                tracing::warn!(
                    "Could not acquire lock to clean up named pipe server: {}",
                    self.path
                );
            }
        }
    }

    impl IpcListener {
        pub async fn accept(&self) -> Result<IpcStream> {
            let mut server_guard = self.current_server.lock().await;

            if let Some(server) = server_guard.take() {
                // Wait for a client to connect
                server.connect().await?;

                // Create a new server instance for the next connection
                let new_server = ServerOptions::new()
                    .first_pipe_instance(false)
                    .in_buffer_size(65536)
                    .out_buffer_size(65536)
                    .create(&self.path)?;
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

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
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
