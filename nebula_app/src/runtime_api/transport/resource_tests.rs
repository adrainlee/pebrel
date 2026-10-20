use super::*;

fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    (client, listener.accept().unwrap().0)
}

#[test]
fn stalled_writer_has_total_deadline() {
    let (_client, mut server) = sockets();
    let deadline = Instant::now() + Duration::from_millis(100);
    let payload = "x".repeat(16 * 1024 * 1024);
    assert!(write_json_line_until(&mut server, &payload, deadline).is_err());
    assert!(deadline.elapsed() < Duration::from_secs(3));
}

#[test]
fn reader_deadline_expires_even_if_bytes_are_ready() {
    let (mut client, server) = sockets();
    client.write_all(b"unfinished").unwrap();
    let mut reader = RequestReader { stream: server, deadline: Instant::now() };
    assert_eq!(reader.read(&mut [0; 1]).unwrap_err().kind(), std::io::ErrorKind::TimedOut);
}

#[test]
fn idle_subscriber_disconnect_releases_registration_without_a_publish() {
    let hub = RuntimeHub::new();
    let (client, mut server) = sockets();
    client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let worker_hub = hub.clone();
    let (done, completion) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let result = subscribe_connection(
            &mut server,
            ApiRequest::new("fixture".into(), "events.subscribe", json!({})),
            &worker_hub,
        );
        done.send(result).unwrap();
    });
    let mut reader = BufReader::new(client);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(serde_json::from_str::<ApiResponse>(&line).unwrap().ok);
    assert_eq!(hub.lock().subscribers.len(), 1);
    drop(reader);
    completion.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
    worker.join().unwrap();
    assert!(hub.lock().subscribers.is_empty());
}
