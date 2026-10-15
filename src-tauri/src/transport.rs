use anyhow::{bail, ensure, Context};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use hyper::{
    body::Incoming,
    header::{CONNECTION, HOST},
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

pub struct Gate {
    open: AtomicBool,
    generation: Mutex<CancellationToken>,
}
impl Gate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            open: AtomicBool::new(false),
            generation: Mutex::new(CancellationToken::new()),
        })
    }
    pub fn ready(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        let mut g = self.generation.lock().expect("gate mutex");
        g.cancel();
        *g = CancellationToken::new();
    }
    fn session(&self) -> Option<CancellationToken> {
        let g = self.generation.lock().expect("gate mutex");
        self.open.load(Ordering::SeqCst).then(|| g.clone())
    }
}

pub async fn socks_connect(proxy: SocketAddr, host: &str, port: u16) -> anyhow::Result<TcpStream> {
    tokio::time::timeout(Duration::from_secs(8), async {
        ensure!(
            !host.is_empty() && host.len() <= 255 && port > 0,
            "invalid SOCKS destination"
        );
        let mut stream = TcpStream::connect(proxy)
            .await
            .context("本地 SOCKS5 未就绪")?;
        stream.write_all(&[5, 1, 0]).await?;
        let mut greeting = [0; 2];
        stream.read_exact(&mut greeting).await?;
        ensure!(greeting == [5, 0], "SOCKS5 不支持无认证连接");
        let mut request = vec![5, 1, 0, 3, host.len() as u8];
        request.extend_from_slice(host.as_bytes());
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await?;
        let mut reply = [0; 4];
        stream.read_exact(&mut reply).await?;
        ensure!(
            reply[0] == 5 && reply[1] == 0 && reply[2] == 0,
            "SOCKS5 连接失败（代码 {}）",
            reply[1]
        );
        let len = match reply[3] {
            1 => 4,
            4 => 16,
            3 => stream.read_u8().await? as usize,
            _ => bail!("SOCKS5 地址类型无效"),
        };
        let mut rest = vec![0; len + 2];
        stream.read_exact(&mut rest).await?;
        Ok(stream)
    })
    .await
    .context("SOCKS5 连接超时")?
}

type Body = BoxBody<Bytes, hyper::Error>;
fn text_response(status: StatusCode, text: &'static str) -> Response<Body> {
    let body = Full::new(Bytes::from_static(text.as_bytes()))
        .map_err(|never| match never {})
        .boxed();
    Response::builder()
        .status(status)
        .body(body)
        .expect("static response")
}

fn strip_hop_headers(headers: &mut hyper::HeaderMap) {
    let named = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|v| v.trim().to_owned())
        .collect::<Vec<_>>();
    for name in named {
        headers.remove(name.as_str());
    }
    for name in [
        "connection",
        "proxy-connection",
        "proxy-authorization",
        "proxy-authenticate",
        "keep-alive",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ] {
        headers.remove(name);
    }
}

async fn forward(
    mut req: Request<Incoming>,
    socks: SocketAddr,
    gate: Arc<Gate>,
    stop: CancellationToken,
) -> Result<Response<Body>, Infallible> {
    let Some(session) = gate.session() else {
        return Ok(text_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "SSH tunnel is reconnecting",
        ));
    };
    if req.method() == Method::CONNECT {
        let Some(authority) = req.uri().authority().cloned() else {
            return Ok(text_response(
                StatusCode::BAD_REQUEST,
                "CONNECT requires host:port",
            ));
        };
        let Some(port) = authority.port_u16() else {
            return Ok(text_response(
                StatusCode::BAD_REQUEST,
                "CONNECT requires a port",
            ));
        };
        let host = authority.host().trim_matches(['[', ']']);
        let connect = tokio::select! {
            _=stop.cancelled()=>return Ok(text_response(StatusCode::SERVICE_UNAVAILABLE,"Proxy stopped")),
            _=session.cancelled()=>return Ok(text_response(StatusCode::SERVICE_UNAVAILABLE,"Tunnel changed")),
            result=socks_connect(socks,host,port)=>result,
        };
        let Ok(mut upstream) = connect else {
            return Ok(text_response(
                StatusCode::BAD_GATEWAY,
                "SSH destination connection failed",
            ));
        };
        let upgrade = hyper::upgrade::on(&mut req);
        tokio::spawn(async move {
            tokio::select! {
                _=stop.cancelled()=>{}, _=session.cancelled()=>{},
                _=async {
                    if let Ok(upgraded)=upgrade.await {
                        let mut client=TokioIo::new(upgraded);
                        let _=tokio::io::copy_bidirectional(&mut client,&mut upstream).await;
                    }
                }=>{}
            }
        });
        return Ok(text_response(StatusCode::OK, ""));
    }
    if req.uri().scheme_str() != Some("http") || req.uri().authority().is_none() {
        return Ok(text_response(
            StatusCode::BAD_REQUEST,
            "Use absolute http:// URL or HTTPS CONNECT",
        ));
    }
    let authority = req.uri().authority().expect("validated authority").clone();
    let host = authority.host().trim_matches(['[', ']']);
    let port = authority.port_u16().unwrap_or(80);
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/")
        .to_owned();
    let operation = async {
        let stream = socks_connect(socks, host, port).await?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
        let driver_stop = stop.clone();
        let driver_session = session.clone();
        tokio::spawn(async move {
            tokio::select! {_=driver_stop.cancelled()=>{},_=driver_session.cancelled()=>{},_=connection=>{}}
        });
        *req.uri_mut() = path.parse()?;
        strip_hop_headers(req.headers_mut());
        req.headers_mut().insert(HOST, authority.as_str().parse()?);
        let mut response = sender.send_request(req).await?;
        strip_hop_headers(response.headers_mut());
        Ok::<_, anyhow::Error>(response.map(BodyExt::boxed))
    };
    let result = tokio::select! {
        _=stop.cancelled()=>return Ok(text_response(StatusCode::SERVICE_UNAVAILABLE,"Proxy stopped")),
        _=session.cancelled()=>return Ok(text_response(StatusCode::SERVICE_UNAVAILABLE,"Tunnel changed")),
        r=tokio::time::timeout(Duration::from_secs(30),operation)=>r,
    };
    Ok(match result {
        Ok(Ok(response)) => response,
        _ => text_response(StatusCode::BAD_GATEWAY, "SSH forwarding failed"),
    })
}

pub async fn serve_bridge(
    listener: TcpListener,
    socks: SocketAddr,
    gate: Arc<Gate>,
    stop: CancellationToken,
) -> anyhow::Result<()> {
    let mut tasks = JoinSet::new();
    let mut failures = 0;
    loop {
        tokio::select! {
            _=stop.cancelled()=>break,
            Some(_)=tasks.join_next(),if !tasks.is_empty()=>{},
            accepted=listener.accept()=>{
                let (stream,_)=match accepted {
                    Ok(client)=>{failures=0;client},
                    Err(e)=>{
                        failures+=1;
                        if failures>=20 {tasks.abort_all();return Err(e.into())}
                        tokio::select!{_=stop.cancelled()=>break,_=tokio::time::sleep(Duration::from_millis(100))=>{}}
                        continue
                    }
                };
                if tasks.len()>=256 {drop(stream);continue}
                let gate=gate.clone();let stop=stop.clone();
                tasks.spawn(async move {
                    let svc_stop=stop.clone();
                    let service=hyper::service::service_fn(move|req|forward(req,socks,gate.clone(),svc_stop.clone()));
                    tokio::select! {
                        _=stop.cancelled()=>{},
                        _=hyper::server::conn::http1::Builder::new().serve_connection(TokioIo::new(stream),service).with_upgrades()=>{}
                    }
                });
            }
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    Ok(())
}
