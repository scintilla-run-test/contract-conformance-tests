use axum::{Json, Router, extract::DefaultBodyLimit, http::StatusCode, routing::post};
use serde_json::{Value, json};

pub const SOURCE_PR_HEAD: &str = "fc6e9052f6f5c03fd4afbe90e25ad8ccb2966493";
pub const MAX_WRITE_BODY_BYTES: usize = 1_048_576;

pub fn router() -> Router {
    return Router::new()
        .route("/v1/forms", post(create_form))
        .layer(DefaultBodyLimit::max(MAX_WRITE_BODY_BYTES));
}

async fn create_form(Json(_form): Json<Value>) -> (StatusCode, Json<Value>) {
    return (StatusCode::CREATED, Json(json!({"ok": true})));
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn oversized_json_is_rejected_before_handler() {
        let oversized_title = "a".repeat(MAX_WRITE_BODY_BYTES);
        let request = Request::builder()
            .method("POST")
            .uri("/v1/forms")
            .header("content-type", "application/json")
            .body(Body::from(format!(
                r#"{{"id":"form_1","title":"{oversized_title}"}}"#
            )))
            .expect("request");
        let response = router().oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn octet_stream_is_rejected_by_json_extractor() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/forms")
            .header("content-type", "application/octet-stream")
            .body(Body::from(r#"{"id":"form_1","title":"Form"}"#))
            .expect("request");
        let response = router().oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }

    #[tokio::test]
    async fn valid_small_json_reaches_handler() {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/forms")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"id":"form_1","title":"Form"}"#))
            .expect("request");
        let response = router().oneshot(request).await.expect("router response");
        assert_eq!(response.status(), StatusCode::CREATED);
    }
}
