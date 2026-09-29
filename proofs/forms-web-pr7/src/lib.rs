use axum::{
    Router,
    extract::Request,
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{Html, IntoResponse, Response},
    routing::get,
};

pub const SOURCE_PR_HEAD: &str = "6638f1dea23c2b181ff1e603c9c56008a38be933";
const JSON_MEDIA_TYPE: &str = "application/json";
const TYPED_FORM_PATH_PREFIX: &str = "/v1/forms/";

pub fn router() -> Router {
    return Router::new()
        .route("/v1/forms/{form_id}", get(|| async { StatusCode::OK }))
        .route("/forms/{form_id}", get(|| async { Html("<form></form>") }))
        .route(
            "/components/forms/{form_id}",
            get(|| async { Html("<form></form>") }),
        )
        .layer(axum::middleware::from_fn(enforce_json_read_accept));
}

async fn enforce_json_read_accept(request: Request, next: Next) -> Response {
    if request.method() != Method::GET || !request.uri().path().starts_with(TYPED_FORM_PATH_PREFIX)
    {
        return next.run(request).await;
    }
    if !accept_headers_allow_json(request.headers()) {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }
    return next.run(request).await;
}

fn accept_headers_allow_json(headers: &HeaderMap) -> bool {
    let values = headers.get_all(header::ACCEPT);
    let mut saw_accept = false;
    let mut best_specificity: Option<u8> = None;
    let mut best_quality = 0.0_f32;

    for value in values.iter() {
        saw_accept = true;
        let Ok(value) = value.to_str() else {
            return false;
        };
        update_json_accept_preference(value, &mut best_specificity, &mut best_quality);
    }
    if !saw_accept {
        return true;
    }
    return best_specificity.is_some() && best_quality > 0.0;
}

fn update_json_accept_preference(
    value: &str,
    best_specificity: &mut Option<u8>,
    best_quality: &mut f32,
) {
    for item in value.split(',') {
        let mut pieces = item.split(';');
        let media_type = pieces
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        let specificity = match media_type.as_str() {
            JSON_MEDIA_TYPE => 2,
            "application/*" => 1,
            "*/*" => 0,
            _ => continue,
        };
        let mut quality = 1.0_f32;
        for parameter in pieces {
            let mut pair = parameter.trim().splitn(2, '=');
            let name = pair.next().unwrap_or_default().trim();
            let raw_value = pair.next().unwrap_or_default().trim();
            if name.eq_ignore_ascii_case("q") {
                quality = match raw_value.parse::<f32>() {
                    Ok(parsed) if (0.0..=1.0).contains(&parsed) => parsed,
                    _ => 0.0,
                };
            }
        }
        match *best_specificity {
            None => {
                *best_specificity = Some(specificity);
                *best_quality = quality;
            }
            Some(current) if specificity > current => {
                *best_specificity = Some(specificity);
                *best_quality = quality;
            }
            Some(current) if specificity == current && quality > *best_quality => {
                *best_quality = quality;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    async fn status(path: &str, accept: &str) -> StatusCode {
        let request = Request::builder()
            .uri(path)
            .header("accept", accept)
            .body(Body::empty())
            .expect("request");
        return router().oneshot(request).await.expect("response").status();
    }

    #[tokio::test]
    async fn typed_read_rejects_binary_only_accept() {
        assert_eq!(
            status("/v1/forms/a", "application/msgpack").await,
            StatusCode::NOT_ACCEPTABLE
        );
        assert_eq!(
            status("/v1/forms/a", "application/cbor").await,
            StatusCode::NOT_ACCEPTABLE
        );
        assert_eq!(
            status("/v1/forms/a", "application/x-protobuf").await,
            StatusCode::NOT_ACCEPTABLE
        );
    }

    #[tokio::test]
    async fn explicit_json_exclusion_beats_wildcard() {
        assert_eq!(
            status("/v1/forms/a", "application/json;q=0, */*;q=1").await,
            StatusCode::NOT_ACCEPTABLE
        );
        assert_eq!(
            status("/v1/forms/a", "application/*;q=0, */*;q=1").await,
            StatusCode::NOT_ACCEPTABLE
        );
    }

    #[tokio::test]
    async fn typed_read_accepts_json() {
        assert_eq!(
            status("/v1/forms/a", "application/json").await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn ssr_and_component_routes_ignore_typed_json_guard() {
        assert_eq!(
            status("/forms/a", "application/msgpack").await,
            StatusCode::OK
        );
        assert_eq!(
            status("/components/forms/a", "application/cbor").await,
            StatusCode::OK
        );
    }
}
