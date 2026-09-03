#[cfg(unix)]
mod platform {
    use std::path::{Path, PathBuf};

    use serde::{Serialize, de::DeserializeOwned};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{UnixListener, UnixStream},
    };
    use zerobeat_protocol::{decode, encode};

    use crate::IpcError;

    const MAX_FRAME_BYTES: usize = 1024 * 1024;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PeerCredentials {
        pub pid: Option<u32>,
        pub uid: u32,
    }

    pub struct IpcConnection {
        stream: UnixStream,
    }

    pub struct IpcListener {
        listener: UnixListener,
        endpoint: PathBuf,
    }

    impl IpcListener {
        pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
            let endpoint = path.as_ref().to_path_buf();
            Ok(Self {
                listener: UnixListener::bind(&endpoint)?,
                endpoint,
            })
        }

        pub async fn accept(&mut self) -> std::io::Result<IpcConnection> {
            let (stream, _) = self.listener.accept().await?;
            Ok(IpcConnection::from_stream(stream))
        }

        pub fn endpoint(&self) -> &Path {
            &self.endpoint
        }
    }

    impl IpcConnection {
        pub async fn connect(path: impl AsRef<Path>) -> Result<Self, IpcError> {
            let stream = UnixStream::connect(path).await?;
            Ok(Self { stream })
        }

        pub fn from_stream(stream: UnixStream) -> Self {
            Self { stream }
        }

        pub fn peer_credentials(&self) -> std::io::Result<PeerCredentials> {
            let credentials = self.stream.peer_cred()?;
            Ok(PeerCredentials {
                pid: credentials.pid().and_then(|pid| u32::try_from(pid).ok()),
                uid: credentials.uid(),
            })
        }

        pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), IpcError> {
            let payload = encode(message)?;
            if payload.len() > MAX_FRAME_BYTES {
                return Err(IpcError::FrameTooLarge(payload.len()));
            }
            self.stream.write_u32(payload.len() as u32).await?;
            self.stream.write_all(&payload).await?;
            self.stream.flush().await?;
            Ok(())
        }

        pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<T, IpcError> {
            let length = self.stream.read_u32().await? as usize;
            if length > MAX_FRAME_BYTES {
                return Err(IpcError::FrameTooLarge(length));
            }
            let mut payload = vec![0; length];
            self.stream.read_exact(&mut payload).await?;
            decode(&payload).map_err(IpcError::from)
        }
    }

    #[cfg(test)]
    mod tests {
        use std::os::unix::fs::MetadataExt;

        use tempfile::tempdir;

        use super::{IpcConnection, IpcListener};

        #[tokio::test]
        async fn peer_credentials_expose_the_connected_process() {
            let directory = tempdir().unwrap();
            let socket = directory.path().join("peer.sock");
            let mut listener = IpcListener::bind(&socket).unwrap();
            let server = tokio::spawn(async move { listener.accept().await.unwrap() });

            let client = IpcConnection::connect(&socket).await.unwrap();
            let credentials = client.peer_credentials().unwrap();
            assert_eq!(credentials.pid, Some(std::process::id()));
            assert_eq!(
                credentials.uid,
                std::fs::metadata(directory.path()).unwrap().uid()
            );
            drop(client);
            drop(server.await.unwrap());
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        hash::{DefaultHasher, Hash, Hasher},
        os::windows::io::AsRawHandle,
        path::{Path, PathBuf},
        time::Duration,
    };

    use serde::{Serialize, de::DeserializeOwned};
    use tokio::{
        io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
        net::windows::named_pipe::{
            ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            },
            GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
        },
        System::{
            Pipes::GetNamedPipeServerProcessId,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };
    use zerobeat_protocol::{decode, encode};

    use crate::IpcError;

    const MAX_FRAME_BYTES: usize = 1024 * 1024;
    const ERROR_PIPE_BUSY: i32 = 231;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct PeerCredentials {
        pub pid: Option<u32>,
        pub uid: u32,
    }

    enum PipeStream {
        Client(NamedPipeClient),
        Server(NamedPipeServer),
    }

    pub struct IpcConnection {
        stream: PipeStream,
    }

    pub struct IpcListener {
        endpoint: PathBuf,
        pending: NamedPipeServer,
    }

    impl IpcListener {
        pub fn bind(path: impl AsRef<Path>) -> std::io::Result<Self> {
            let endpoint = named_pipe_path(path.as_ref());
            let pending = create_server(&endpoint, true)?;
            Ok(Self { endpoint, pending })
        }

        pub async fn accept(&mut self) -> std::io::Result<IpcConnection> {
            self.pending.connect().await?;
            let next = create_server(&self.endpoint, false)?;
            let connected = std::mem::replace(&mut self.pending, next);
            Ok(IpcConnection {
                stream: PipeStream::Server(connected),
            })
        }

        pub fn endpoint(&self) -> &Path {
            &self.endpoint
        }
    }

    fn create_server(path: &Path, first: bool) -> std::io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        let mut security = current_user_pipe_security()?;
        unsafe {
            options.create_with_security_attributes_raw(
                path,
                (&mut security.attributes as *mut SECURITY_ATTRIBUTES).cast(),
            )
        }
    }

    struct PipeSecurity {
        descriptor: *mut core::ffi::c_void,
        attributes: SECURITY_ATTRIBUTES,
    }

    impl Drop for PipeSecurity {
        fn drop(&mut self) {
            unsafe { LocalFree(self.descriptor) };
        }
    }

    fn current_user_pipe_security() -> std::io::Result<PipeSecurity> {
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let result = current_user_pipe_security_from_token(token);
        unsafe { CloseHandle(token) };
        result
    }

    fn current_user_pipe_security_from_token(
        token: windows_sys::Win32::Foundation::HANDLE,
    ) -> std::io::Result<PipeSecurity> {
        let mut length = 0_u32;
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut length) };
        if length == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut token_info = vec![0_u8; length as usize];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                token_info.as_mut_ptr().cast(),
                length,
                &mut length,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let user = unsafe { &*(token_info.as_ptr().cast::<TOKEN_USER>()) };
        let mut sid_text = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid_text) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let sid = unsafe {
            let mut length = 0;
            while *sid_text.add(length) != 0 {
                length += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(sid_text, length))
        };
        unsafe { LocalFree(sid_text.cast()) };

        let sddl = format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)");
        let wide = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(PipeSecurity {
            descriptor,
            attributes: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            },
        })
    }

    impl IpcConnection {
        pub async fn connect(path: impl AsRef<Path>) -> Result<Self, IpcError> {
            let path = named_pipe_path(path.as_ref());
            for _ in 0..40 {
                match ClientOptions::new().open(&path) {
                    Ok(stream) => {
                        return Ok(Self {
                            stream: PipeStream::Client(stream),
                        });
                    }
                    Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "timed out waiting for ZeroBeat named pipe",
            )
            .into())
        }

        pub fn peer_credentials(&self) -> std::io::Result<PeerCredentials> {
            let PipeStream::Client(stream) = &self.stream else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "server-side named pipe has no daemon credentials",
                ));
            };
            let mut pid = 0_u32;
            let result =
                unsafe { GetNamedPipeServerProcessId(stream.as_raw_handle().cast(), &mut pid) };
            if result == 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(PeerCredentials {
                pid: Some(pid),
                uid: 0,
            })
        }

        pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), IpcError> {
            let payload = encode(message)?;
            if payload.len() > MAX_FRAME_BYTES {
                return Err(IpcError::FrameTooLarge(payload.len()));
            }
            match &mut self.stream {
                PipeStream::Client(stream) => write_frame(stream, &payload).await,
                PipeStream::Server(stream) => write_frame(stream, &payload).await,
            }
        }

        pub async fn receive<T: DeserializeOwned>(&mut self) -> Result<T, IpcError> {
            let payload = match &mut self.stream {
                PipeStream::Client(stream) => read_frame(stream).await?,
                PipeStream::Server(stream) => read_frame(stream).await?,
            };
            decode(&payload).map_err(IpcError::from)
        }
    }

    fn named_pipe_path(path: &Path) -> PathBuf {
        if path
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(r"\\.\pipe\")
        {
            return path.to_path_buf();
        }
        let mut hasher = DefaultHasher::new();
        path.hash(&mut hasher);
        PathBuf::from(format!(r"\\.\pipe\ZeroBeat-test-{:016x}", hasher.finish()))
    }

    async fn write_frame(
        stream: &mut (impl AsyncWrite + Unpin),
        payload: &[u8],
    ) -> Result<(), IpcError> {
        stream.write_u32(payload.len() as u32).await?;
        stream.write_all(payload).await?;
        stream.flush().await?;
        Ok(())
    }

    async fn read_frame(stream: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>, IpcError> {
        let length = stream.read_u32().await? as usize;
        if length > MAX_FRAME_BYTES {
            return Err(IpcError::FrameTooLarge(length));
        }
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).await?;
        Ok(payload)
    }
}

pub use platform::{IpcConnection, IpcListener, PeerCredentials};
