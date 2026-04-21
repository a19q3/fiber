use fnn::x402::{settle_with_rpc, supported_response, FacilitatorRequest, PAYMENT_RESPONSE_HEADER};
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::header::{HeaderValue, CONTENT_TYPE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::env;
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let rpc_url = env::var("FIBER_X402_RPC_URL").unwrap_or_else(|_| "http://127.0.0.1:8227".to_string());
    let listen_addr: SocketAddr = env::var("FIBER_X402_LISTEN_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:4021".to_string())
        .parse()?;
    let rpc = fnn::x402::FiberRpcClient::try_new(&rpc_url)?;
    let listener = TcpListener::bind(listen_addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let rpc = rpc.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |request| handle_request(request, rpc.clone()));
            if let Err(err) = http1::Builder::new().serve_connection(io, service).await {
                tracing::error!(?err, "x402 facilitator connection error");
            }
        });
    }
}

async fn handle_request(
    request: Request<hyper::body::Incoming>,
    rpc: fnn::x402::FiberRpcClient,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let response = match (request.method(), request.uri().path()) {
        (&Method::GET, "/supported") => json_response(StatusCode::OK, &supported_response(), None),
        (&Method::POST, "/settle") => match request.into_body().collect().await {
            Ok(collected) => match serde_json::from_slice::<FacilitatorRequest>(&collected.to_bytes()) {
                Ok(payload) => match settle_with_rpc(&rpc, payload).await {
                    Ok(settlement) => {
                        let header = serde_json::to_string(&settlement).ok();
                        json_response(StatusCode::OK, &settlement, header)
                    }
                    Err(err) => json_response(
                        StatusCode::BAD_REQUEST,
                        &serde_json::json!({ "success": false, "errorReason": err.to_string() }),
                        None,
                    ),
                },
                Err(err) => json_response(
                    StatusCode::BAD_REQUEST,
                    &serde_json::json!({ "success": false, "errorReason": err.to_string() }),
                    None,
                ),
            },
            Err(err) => json_response(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({ "success": false, "errorReason": err.to_string() }),
                None,
            ),
        },
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::new(Bytes::new()))
            .unwrap(),
    };
    Ok(response)
}

fn json_response<T: serde::Serialize>(
    status: StatusCode,
    body: &T,
    payment_response_header: Option<String>,
) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(body).unwrap();
    let mut builder = Response::builder().status(status).header(CONTENT_TYPE, "application/json");
    if let Some(value) = payment_response_header {
        if let Ok(value) = HeaderValue::from_str(&value) {
            builder = builder.header(PAYMENT_RESPONSE_HEADER, value);
        }
    }
    builder.body(Full::new(Bytes::from(body))).unwrap()
}
