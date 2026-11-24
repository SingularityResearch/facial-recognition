//! One local web server: 127.0.0.1 only, every request through the checks in `guard`, every answer with the
//! page headers and never to be kept by a browser. A program's routes are a `Service`; this file is the
//! transport, so the same routes can later be reached without a socket (the app window's own channel).
//!
//! Found out the hard way, and kept: connections stay open (a new one per request failed about once
//! in a thousand on Windows), an answer leaves in one piece and at once (no waiting for an acknowledgement:
//! 40 ms a request on Linux), and a second copy fails to bind instead of sharing the port.

use std::{collections::HashMap, convert::Infallible, io, sync::Arc};

use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    HeaderMap, Method, StatusCode,
    body::{Bytes, Incoming},
    header::{self, HeaderValue},
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpListener;

use crate::guard;

/// A request as a program's routes see it.
pub struct Request {
    pub path: String,
    pub query: String,
    /// by lower-case name
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    /// Every value the query string has under a name, decoded, the empty ones left out.
    pub fn params(&self, key: &str) -> Vec<String> {
        (self.query.split('&'))
            .filter_map(|pair| {
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                (unescape(k) == key && !v.is_empty()).then(|| unescape(v))
            })
            .collect()
    }

    /// A value of the query string, decoded.
    pub fn param(&self, key: &str) -> Option<String> {
        self.query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (unescape(k) == key).then(|| unescape(v))
        })
    }
}

fn unescape(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub struct Response {
    pub status: u16,
    pub ctype: &'static str,
    pub body: Vec<u8>,
}

impl Response {
    pub fn json(value: Value) -> Response {
        Response { status: 200, ctype: "application/json", body: value.to_string().into_bytes() }
    }
    pub fn error(status: u16, why: impl Into<String>) -> Response {
        Response { status, ctype: "application/json", body: json!({"error": why.into()}).to_string().into_bytes() }
    }
    pub fn bytes(ctype: &'static str, body: Vec<u8>) -> Response {
        Response { status: 200, ctype, body }
    }
    pub fn not_found() -> Response {
        Response { status: 404, ctype: "text/plain", body: b"not found".to_vec() }
    }
}

/// A program's routes.
pub trait Service: Send + Sync + 'static {
    /// what this program's pages need beyond the shared policy
    fn policy(&self) -> &'static [(&'static str, &'static str)] {
        &[]
    }
    /// pages allowed to show this program's pages in a frame (the app's tab page, when it started the program)
    fn frame_parents(&self) -> Vec<String> {
        guard::frame_parents()
    }
    /// the whole Content-Security-Policy of this program's answers
    fn page_policy(&self) -> String {
        guard::policy(self.policy(), &self.frame_parents())
    }
    /// which pages may use the camera and microphone through this one (the app's tab page says)
    fn permissions(&self) -> Option<String> {
        None
    }
    /// For a GET: (it does something, so it needs the page's own header; it gives data or starts work, so
    /// only the program's own page may send it).
    fn get_rules(&self, path: &str) -> (bool, bool) {
        (false, guard::OWN_ONLY.iter().any(|p| path.starts_with(p)))
    }
    /// For a POST: one more than the most bytes its body may have; None when there is no such route.
    fn body_limit(&self, path: &str) -> Option<usize>;
    fn get(&self, req: &Request) -> Response;
    fn post(&self, req: &Request) -> Response;
}

/// A socket on 127.0.0.1:port, not serving yet. An error when the port is taken (on Windows too, where a
/// second program could otherwise share it).
pub fn bind(port: u16) -> io::Result<std::net::TcpListener> {
    let l = std::net::TcpListener::bind(("127.0.0.1", port))?;
    l.set_nonblocking(true)?;
    Ok(l)
}

/// Serve until the task is dropped.
pub async fn serve<S: Service>(listener: std::net::TcpListener, service: Arc<S>) -> io::Result<()> {
    let port = listener.local_addr()?.port();
    let listener = TcpListener::from_std(listener)?;
    loop {
        let (stream, _) = match listener.accept().await {
            Ok(s) => s,
            Err(_) => continue,
        };
        let _ = stream.set_nodelay(true);
        let service = service.clone();
        tokio::spawn(async move {
            let handler = service_fn(move |req| {
                let service = service.clone();
                async move { Ok::<_, Infallible>(answer(req, service, port).await) }
            });
            let mut http = hyper::server::conn::http1::Builder::new();
            http.keep_alive(true);
            let _ = http.serve_connection(TokioIo::new(stream), handler).await;
        });
    }
}

fn lower(headers: &HeaderMap) -> HashMap<String, String> {
    headers.iter().filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string()))).collect()
}

async fn answer<S: Service>(req: hyper::Request<Incoming>, service: Arc<S>, port: u16) -> hyper::Response<Full<Bytes>> {
    let (parts, body) = req.into_parts();
    let headers = lower(&parts.headers);
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().unwrap_or("").to_string();
    let write = parts.method == Method::POST;
    let finish = |r: Response, close: bool| finish(r, &*service, close);
    if !write && parts.method != Method::GET {
        return finish(Response::error(405, "not allowed"), true);
    }
    let (acts, own) = if write { (true, false) } else { service.get_rules(&path) };
    if let Some(why) = guard::refuse_reason(&headers, port, acts, own) {
        return finish(Response::error(403, why), true);
    }
    let mut request = Request { path, query, headers, body: Vec::new() };
    if write {
        let Some(limit) = service.body_limit(&request.path) else {
            return finish(Response::not_found(), true);
        };
        if request.headers.contains_key("transfer-encoding") {
            return finish(Response::error(400, "bad Content-Length"), true);
        }
        match guard::body_size(request.headers.get("content-length").map(String::as_str), limit) {
            Err((status, why)) => return finish(Response::error(status, why), true),
            Ok(_) => match Limited::new(body, limit).collect().await {
                Ok(b) => request.body = b.to_bytes().to_vec(),
                Err(_) => return finish(Response::error(400, "the request's body didn't arrive"), true),
            },
        }
    }
    // the routes run models and read files: on a thread of their own, never on the ones that take connections
    let work = tokio::task::spawn_blocking(move || {
        let r = if write { service.post(&request) } else { service.get(&request) };
        (r, service)
    });
    match work.await {
        Ok((r, service)) => self::finish(r, &*service, false),
        Err(e) => {
            eprintln!("A request failed: {e}");
            let mut r = hyper::Response::new(Full::new(Bytes::from(
                json!({"error": "the program failed on this request"}).to_string(),
            )));
            *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
            r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
            r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
    }
}

/// The answer as it leaves: its length, never kept by a browser, and the page headers (what the page may
/// load and who may frame it, no content sniffing, no reading by another site's page, and each server's
/// pages in a process of their own in a Chromium window).
fn finish<S: Service>(r: Response, service: &S, close: bool) -> hyper::Response<Full<Bytes>> {
    let mut out = hyper::Response::new(Full::new(Bytes::from(r.body)));
    *out.status_mut() = StatusCode::from_u16(r.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let h = out.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(r.ctype));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&service.page_policy()) {
        h.insert(header::CONTENT_SECURITY_POLICY, v);
    }
    if let Some(Ok(v)) = service.permissions().map(|p| HeaderValue::from_str(&p)) {
        h.insert("permissions-policy", v);
    }
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert("origin-agent-cluster", HeaderValue::from_static("?1"));
    if close {
        h.insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_values_are_decoded() {
        let r = Request {
            path: "/api/x".into(),
            query: "t=0.35&name=A%20B+C&c=1&bad=%zz&end=%".into(),
            headers: HashMap::new(),
            body: vec![],
        };
        assert_eq!(r.param("t").as_deref(), Some("0.35"));
        assert_eq!(r.param("name").as_deref(), Some("A B C"));
        assert_eq!(r.param("bad").as_deref(), Some("%zz"));
        assert_eq!(r.param("end").as_deref(), Some("%"));
        assert_eq!(r.param("missing"), None);
    }
}
