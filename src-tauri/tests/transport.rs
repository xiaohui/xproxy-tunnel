use std::net::SocketAddr;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use tokio_util::sync::CancellationToken;
use xproxy::transport::{serve_bridge, socks_connect, Gate};

async fn socks_fixture() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = l.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut greeting = [0; 3];
                s.read_exact(&mut greeting).await.unwrap();
                assert_eq!(greeting, [5, 1, 0]);
                s.write_all(&[5, 0]).await.unwrap();
                let mut head = [0; 5];
                s.read_exact(&mut head).await.unwrap();
                assert_eq!(&head[..4], &[5, 1, 0, 3]);
                let mut host = vec![0; head[4] as usize];
                s.read_exact(&mut host).await.unwrap();
                let port = s.read_u16().await.unwrap();
                let host = String::from_utf8(host).unwrap();
                if let Ok(mut remote) = TcpStream::connect((host.as_str(), port)).await {
                    s.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                        .await
                        .unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut s, &mut remote).await;
                } else {
                    let _ = s.write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0]).await;
                }
            });
        }
    });
    addr
}

#[tokio::test]
async fn socks_handshake_routes_bytes_and_rejects_refusal() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut s, _) = echo.accept().await.unwrap();
        let mut b = [0; 4];
        s.read_exact(&mut b).await.unwrap();
        s.write_all(&b).await.unwrap();
    });
    let proxy = socks_fixture().await;
    let mut s = socks_connect(proxy, "127.0.0.1", port).await.unwrap();
    s.write_all(b"ping").await.unwrap();
    let mut b = [0; 4];
    s.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"ping");
    assert!(socks_connect(proxy, "127.0.0.1", port).await.is_err());
}

#[tokio::test]
async fn http_bridge_routes_http_and_connect_and_blocks_unhealthy_tunnel() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin_port = origin.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = origin.accept().await.unwrap();
            tokio::spawn(async move {
                let mut bytes = vec![0; 4096];
                let n = s.read(&mut bytes).await.unwrap();
                let req = String::from_utf8_lossy(&bytes[..n]);
                if req.starts_with("GET /hello") {
                    s.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await
                    .unwrap();
                } else {
                    s.write_all(&bytes[..n]).await.unwrap();
                }
            });
        }
    });
    let socks = socks_fixture().await;
    let bridge = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = bridge.local_addr().unwrap();
    let gate = Gate::new();
    let stop = CancellationToken::new();
    let task = tokio::spawn(serve_bridge(bridge, socks, gate.clone(), stop.clone()));
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http(format!("http://{addr}")).unwrap())
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let url = format!("http://127.0.0.1:{origin_port}/hello");
    assert_eq!(client.get(&url).send().await.unwrap().status(), 503);
    gate.ready();
    assert_eq!(
        client.get(&url).send().await.unwrap().text().await.unwrap(),
        "hello"
    );
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(
            format!(
                "CONNECT 127.0.0.1:{origin_port} HTTP/1.1\r\nHost: 127.0.0.1:{origin_port}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = vec![0; 1024];
    let n = socket.read(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response[..n]).starts_with("HTTP/1.1 200"));
    socket.write_all(b"ping").await.unwrap();
    let mut b = [0; 4];
    socket.read_exact(&mut b).await.unwrap();
    assert_eq!(&b, b"ping");
    gate.close();
    assert_eq!(client.get(&url).send().await.unwrap().status(), 503);
    stop.cancel();
    task.await.unwrap().unwrap();
    assert!(TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn closing_gate_terminates_an_inflight_connect_tunnel() {
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = echo.local_addr().unwrap().port();
    let echo_task = tokio::spawn(async move {
        let (mut s, _) = echo.accept().await.unwrap();
        let (mut read, mut write) = s.split();
        tokio::io::copy(&mut read, &mut write).await.unwrap();
    });
    let socks = socks_fixture().await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let gate = Gate::new();
    gate.ready();
    let stop = CancellationToken::new();
    let task = tokio::spawn(serve_bridge(listener, socks, gate.clone(), stop.clone()));
    let mut client = TcpStream::connect(addr).await.unwrap();
    client
        .write_all(
            format!("CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut header = [0; 1024];
    let n = client.read(&mut header).await.unwrap();
    assert!(String::from_utf8_lossy(&header[..n]).starts_with("HTTP/1.1 200"));
    client.write_all(b"ping").await.unwrap();
    let mut bytes = [0; 4];
    client.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"ping");
    gate.close();
    let n = tokio::time::timeout(std::time::Duration::from_secs(1), client.read(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 0);
    stop.cancel();
    task.await.unwrap().unwrap();
    echo_task.await.unwrap();
}
