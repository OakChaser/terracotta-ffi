use crate::scaffolding::{PacketResponse, TIMEOUT};
use socket2::{Domain, SockAddr, Socket, Type};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::{io, thread};

pub type HandleFunction =
    Box<dyn Fn(&[u8], Vec<u8>) -> io::Result<PacketResponse> + Send + Sync>;
pub type Handler = (&'static str, &'static str, HandleFunction);
pub type Handlers = Vec<Handler>;
pub type HandleRef<'a> = &'a dyn Fn(&[u8], Vec<u8>) -> io::Result<PacketResponse>;

pub fn start(handlers: Handlers, port: u16) -> io::Result<(u16, ServerHandle)> {
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None).unwrap();
    socket.bind(&SockAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port)))?;
    socket.set_read_timeout(Some(TIMEOUT)).unwrap();
    socket.set_write_timeout(Some(TIMEOUT)).unwrap();
    socket.listen(128)?;

    let port = socket.local_addr().unwrap().as_socket().unwrap().port();

    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_check = Arc::clone(&shutdown);
    let handlers = Arc::new(handlers);
    thread::spawn(move || {
        let listener: TcpListener = socket.into();
        loop {
            if shutdown_check.load(Ordering::Acquire) {
                break;
            }
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    logging!("ScaffoldingServer", "Accept failed: {:?}", e);
                    continue;
                }
            };
            if shutdown_check.load(Ordering::Acquire) {
                break;
            }
            let handlers = Arc::clone(&handlers);
            thread::spawn(move || {
                loop {
                    if let Err(e) = handle_connection(&mut stream, &handlers) {
                        logging!("ScaffoldingServer", "Connection closed: {:?}", e);
                        return;
                    }
                }
            });
        }
    });

    Ok((port, ServerHandle { shutdown, port }))
}

#[derive(Clone)]
pub struct ServerHandle {
    shutdown: Arc<AtomicBool>,
    port: u16,
}

impl ServerHandle {
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        // Unblock a pending accept() so the accept loop notices promptly
        // without waiting for the accept timeout to elapse.
        let _ = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, self.port));
    }
}

fn handle_connection(stream: &mut TcpStream, handlers: &[Handler]) -> io::Result<()> {
    let mut kind_size = [0u8; 1];
    stream.read_exact(&mut kind_size)?;
    let kind_size = kind_size[0] as usize;

    let mut kind = vec![0u8; kind_size];
    stream.read_exact(&mut kind)?;
    let kind = String::from_utf8(kind).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let kinds = kind.splitn(3, ':').collect::<Vec<_>>();
    if kinds.len() != 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid request kind."));
    }

    let mut body_size = [0u8; 4];
    stream.read_exact(&mut body_size)?;
    let body_size = u32::from_be_bytes(body_size) as usize;

    let mut body = vec![0u8; body_size];
    stream.read_exact(&mut body)?;

    fn default_handle(_: &[u8], mut response: Vec<u8>) -> io::Result<PacketResponse> {
        response.extend_from_slice("Requested protocol hasn't been implemented.".as_bytes());
        PacketResponse::fail(255, response)
    }
    let handle: HandleRef = handlers
        .iter()
        .find(|(namespace, path, _)| kinds[0] == *namespace && kinds[1] == *path)
        .map(|(_, _, handle)| handle.as_ref())
        .unwrap_or(&default_handle);

    let mut response = Vec::with_capacity(64);
    response.resize(5, 0u8);

    let (code, mut response) = match handle(&body, response) {
        Ok(PacketResponse::Ok { data }) => (0, data),
        Ok(PacketResponse::Fail { status, data }) => (status, data),
        Err(e) => {
            let mut response = Vec::with_capacity(64);
            response.resize(5, 0u8);
            response.extend_from_slice(format!("{:?}", e).as_bytes());
            (255, response)
        }
    };

    response[0] = code;
    let response_size = (response.len() - 5) as u32;
    response[1..5].copy_from_slice(&response_size.to_be_bytes());

    stream.write_all(&response)?;
    stream.flush()?;
    Ok(())
}
