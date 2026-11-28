use super::*;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// Eight workers fetching many small files from one host open at most eight connections, one per
/// worker, rather than a new one for nearly every file.
#[test]
fn downloads_reuse_their_connections() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/object", listener.local_addr().unwrap());
    let accepted = std::sync::Arc::new(AtomicUsize::new(0));
    let count = accepted.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            count.fetch_add(1, Relaxed);
            std::thread::spawn(move || {
                let mut request = [0; 4096];
                while stream.read(&mut request).is_ok_and(|n| n > 0) {
                    let sent = stream.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: keep-alive\r\n\r\nbody",
                    );
                    if sent.is_err() {
                        break;
                    }
                }
            });
        }
    });
    std::thread::scope(|scope| {
        for _ in 0..DOWNLOAD_JOBS {
            scope.spawn(|| {
                for _ in 0..5 {
                    assert_eq!(get_bytes(&url).unwrap(), b"body");
                }
            });
        }
    });
    assert!(
        accepted.load(Relaxed) <= DOWNLOAD_JOBS,
        "{}",
        accepted.load(Relaxed)
    );
}
