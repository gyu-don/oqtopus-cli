use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct Response {
    content_type: String,
    body: Vec<u8>,
}

/// A loopback-only HTTP fixture server for implementation-neutral CLI tests.
pub struct HttpFixtureServer {
    base_url: Option<String>,
    routes: Arc<Mutex<HashMap<String, Response>>>,
    requests: Arc<Mutex<Vec<String>>>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl HttpFixtureServer {
    pub fn new() -> Self {
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            // Some development sandboxes deny all socket syscalls. Tests also expose
            // file fixtures, so native candidates retain an implementation-neutral seam.
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                return Self {
                    base_url: None,
                    routes,
                    requests,
                    stopping,
                    thread: None,
                };
            }
            Err(error) => panic!("bind HTTP fixture server: {error}"),
        };
        listener
            .set_nonblocking(true)
            .expect("make HTTP fixture listener nonblocking");
        let address = listener.local_addr().expect("read HTTP fixture address");
        let server_routes = Arc::clone(&routes);
        let server_requests = Arc::clone(&requests);
        let server_stopping = Arc::clone(&stopping);
        let thread = thread::spawn(move || {
            while !server_stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => handle(stream, &server_routes, &server_requests),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept HTTP fixture request: {error}"),
                }
            }
        });

        Self {
            base_url: Some(format!("http://{address}")),
            routes,
            requests,
            stopping,
            thread: Some(thread),
        }
    }

    pub fn respond(
        &self,
        path_and_query: impl Into<String>,
        content_type: impl Into<String>,
        body: impl AsRef<[u8]>,
    ) {
        self.routes
            .lock()
            .expect("lock HTTP fixture routes")
            .insert(
                path_and_query.into(),
                Response {
                    content_type: content_type.into(),
                    body: body.as_ref().to_vec(),
                },
            );
    }

    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref()
    }

    pub fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("lock HTTP fixture requests")
            .len()
    }
}

impl Drop for HttpFixtureServer {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(base_url) = &self.base_url {
            let _ = TcpStream::connect(base_url.trim_start_matches("http://"));
        }
        if let Some(thread) = self.thread.take() {
            thread.join().expect("join HTTP fixture server");
        }
    }
}

fn handle(
    mut stream: TcpStream,
    routes: &Mutex<HashMap<String, Response>>,
    requests: &Mutex<Vec<String>>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set HTTP fixture read timeout");
    let mut bytes = [0_u8; 8192];
    let length = stream.read(&mut bytes).expect("read HTTP fixture request");
    let request = String::from_utf8_lossy(&bytes[..length]);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    requests
        .lock()
        .expect("lock HTTP fixture requests")
        .push(format!("{method} {target}"));

    let routes = routes.lock().expect("lock HTTP fixture routes");
    let (status, content_type, body) = match routes.get(target) {
        Some(response) if method == "GET" => (
            "200 OK",
            response.content_type.as_str(),
            response.body.as_slice(),
        ),
        _ => ("404 Not Found", "text/plain", &b"fixture not found"[..]),
    };
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write HTTP fixture response headers");
    stream
        .write_all(body)
        .expect("write HTTP fixture response body");
}
